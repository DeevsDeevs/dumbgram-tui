use color_eyre::Result;
use grammers_session::Session;
use std::path::Path;

#[cfg(any(target_os = "linux", target_os = "macos"))]
mod unix {
    use super::*;
    use exacl::getfacl;
    use rustix::{
        fs::{
            AtFlags, FileType, Mode, OFlags, RenameFlags, fchmod, fstat, fsync, mkdirat, openat,
            renameat, renameat_with, statat, unlinkat,
        },
        process::geteuid,
    };
    use std::{
        ffi::{OsStr, OsString},
        fs::{self, File},
        io::{Read, Seek, SeekFrom, Write},
        os::fd::OwnedFd,
        path::{Component, PathBuf},
        sync::atomic::{AtomicU64, Ordering},
    };

    const PRIVATE_DIR_MODE: Mode = Mode::from_raw_mode(0o700);
    const PRIVATE_FILE_MODE: Mode = Mode::from_raw_mode(0o600);
    const MAX_STAGE_ATTEMPTS: u64 = 100;
    static STAGE_COUNTER: AtomicU64 = AtomicU64::new(0);

    pub struct SecureSessionFile {
        parent_fd: File,
        parent_path: PathBuf,
        leaf: OsString,
    }

    impl SecureSessionFile {
        pub fn canonical_path(&self) -> PathBuf {
            self.parent_path.join(&self.leaf)
        }
    }

    struct StagedFile<'a> {
        owner: &'a SecureSessionFile,
        dir_fd: File,
        dir_name: OsString,
        file: File,
        published: bool,
    }

    impl SecureSessionFile {
        pub fn open(path: &Path) -> Result<(Self, Session)> {
            let leaf = path
                .file_name()
                .filter(|name| !name.is_empty())
                .ok_or_else(|| color_eyre::eyre::eyre!("session path must name a file"))?
                .to_os_string();
            let parent = path
                .parent()
                .filter(|path| !path.as_os_str().is_empty())
                .unwrap_or(Path::new("."));
            let (parent_fd, parent_path) = secure_parent(parent)?;
            let storage = Self {
                parent_fd,
                parent_path,
                leaf,
            };
            let session = match storage.load_existing()? {
                Some(session) => session,
                None => storage.create()?,
            };
            Ok((storage, session))
        }

        pub fn save(&self, session: &Session) -> Result<()> {
            self.validate_existing()?;
            let mut staged = self.stage(&session.save())?;
            renameat(&staged.dir_fd, "session", &self.parent_fd, &self.leaf)?;
            staged.published = true;
            fsync(&staged.dir_fd)?;
            self.verify_published(&staged.file)?;
            fsync(&self.parent_fd)?;
            Ok(())
        }

        fn create(&self) -> Result<Session> {
            let session = Session::new();
            let mut staged = self.stage(&session.save())?;
            match renameat_with(
                &staged.dir_fd,
                "session",
                &self.parent_fd,
                &self.leaf,
                RenameFlags::NOREPLACE,
            ) {
                Ok(()) => {
                    staged.published = true;
                    fsync(&staged.dir_fd)?;
                    self.verify_published(&staged.file)?;
                    fsync(&self.parent_fd)?;
                    Ok(session)
                }
                Err(error) if error == rustix::io::Errno::EXIST => {
                    self.load_existing()?.ok_or_else(|| {
                        color_eyre::eyre::eyre!("session file disappeared during creation")
                    })
                }
                Err(error) => Err(error.into()),
            }
        }

        fn load_existing(&self) -> Result<Option<Session>> {
            let fd = match openat(
                &self.parent_fd,
                &self.leaf,
                OFlags::RDWR | OFlags::CLOEXEC | OFlags::NOFOLLOW,
                Mode::empty(),
            ) {
                Ok(fd) => fd,
                Err(error) if error == rustix::io::Errno::NOENT => return Ok(None),
                Err(error) => return Err(error.into()),
            };
            let mut file = File::from(fd);
            secure_regular_file(&file, &self.parent_path.join(&self.leaf))?;
            let mut bytes = Vec::new();
            file.read_to_end(&mut bytes)?;
            Ok(Some(Session::load(&bytes).map_err(|error| {
                color_eyre::eyre::eyre!("failed to load secure session file: {error}")
            })?))
        }

        fn validate_existing(&self) -> Result<()> {
            self.load_existing()?
                .ok_or_else(|| color_eyre::eyre::eyre!("session file disappeared before save"))?;
            Ok(())
        }

        fn stage(&self, bytes: &[u8]) -> Result<StagedFile<'_>> {
            self.stage_with_failure(bytes, None)
        }

        fn stage_with_failure(
            &self,
            bytes: &[u8],
            #[cfg_attr(not(test), allow(unused_variables))] failure: Option<StageFailure>,
        ) -> Result<StagedFile<'_>> {
            let (dir_name, dir_fd, dir_path) = self.create_stage_dir()?;
            let fd = match openat(
                &dir_fd,
                "session",
                OFlags::CREATE | OFlags::EXCL | OFlags::RDWR | OFlags::CLOEXEC | OFlags::NOFOLLOW,
                PRIVATE_FILE_MODE,
            ) {
                Ok(fd) => fd,
                Err(error) => {
                    let _ = unlinkat(&self.parent_fd, &dir_name, AtFlags::REMOVEDIR);
                    return Err(error.into());
                }
            };
            let mut file = File::from(fd);
            let result = (|| -> Result<()> {
                fchmod(&file, PRIVATE_FILE_MODE)?;
                file.write_all(bytes)?;
                #[cfg(test)]
                if failure == Some(StageFailure::AfterWrite) {
                    color_eyre::eyre::bail!("injected stage failure after write");
                }
                file.sync_all()?;
                #[cfg(test)]
                if failure == Some(StageFailure::AfterSync) {
                    color_eyre::eyre::bail!("injected stage failure after sync");
                }
                file.seek(SeekFrom::Start(0))?;
                #[cfg(test)]
                if failure == Some(StageFailure::BeforeVerify) {
                    color_eyre::eyre::bail!("injected stage failure before verify");
                }
                secure_regular_file(&file, &dir_path.join("session"))?;
                fsync(&dir_fd)?;
                Ok(())
            })();
            if let Err(error) = result {
                let _ = unlinkat(&dir_fd, "session", AtFlags::empty());
                let _ = unlinkat(&self.parent_fd, &dir_name, AtFlags::REMOVEDIR);
                return Err(error);
            }
            Ok(StagedFile {
                owner: self,
                dir_fd,
                dir_name,
                file,
                published: false,
            })
        }

        fn create_stage_dir(&self) -> Result<(OsString, File, PathBuf)> {
            for _ in 0..MAX_STAGE_ATTEMPTS {
                let counter = STAGE_COUNTER.fetch_add(1, Ordering::Relaxed);
                let name = OsString::from(format!(
                    ".dumbgram-session-{}-{counter}",
                    std::process::id()
                ));
                match mkdirat(&self.parent_fd, &name, PRIVATE_DIR_MODE) {
                    Ok(()) => {
                        let path = self.parent_path.join(&name);
                        let fd = open_created_directory(&self.parent_fd, &name, &path)?;
                        return Ok((name, fd, path));
                    }
                    Err(error) if error == rustix::io::Errno::EXIST => continue,
                    Err(error) => return Err(error.into()),
                }
            }
            color_eyre::eyre::bail!("could not reserve a private session staging directory")
        }

        fn verify_published(&self, file: &File) -> Result<()> {
            let path = self.parent_path.join(&self.leaf);
            secure_regular_file(file, &path)?;
            let open_stat = fstat(file)?;
            let published_stat = statat(&self.parent_fd, &self.leaf, AtFlags::SYMLINK_NOFOLLOW)?;
            if !same_identity(&open_stat, &published_stat) {
                color_eyre::eyre::bail!("published session identity changed")
            }
            Ok(())
        }
    }

    #[cfg_attr(not(test), allow(dead_code))]
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum StageFailure {
        AfterWrite,
        AfterSync,
        BeforeVerify,
    }

    impl Drop for StagedFile<'_> {
        fn drop(&mut self) {
            if !self.published {
                let _ = unlinkat(&self.dir_fd, "session", AtFlags::empty());
            }
            let _ = unlinkat(&self.owner.parent_fd, &self.dir_name, AtFlags::REMOVEDIR);
        }
    }

    fn secure_parent(parent: &Path) -> Result<(File, PathBuf)> {
        let absolute = if parent.is_absolute() {
            parent.to_path_buf()
        } else {
            std::env::current_dir()?.join(parent)
        };
        let mut existing = absolute.as_path();
        let mut missing = Vec::new();
        loop {
            match fs::symlink_metadata(existing) {
                Ok(_) => break,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    let name = existing.file_name().ok_or_else(|| {
                        color_eyre::eyre::eyre!("session parent has no existing ancestor")
                    })?;
                    missing.push(name.to_os_string());
                    existing = existing.parent().ok_or_else(|| {
                        color_eyre::eyre::eyre!("session parent has no existing ancestor")
                    })?;
                }
                Err(error) => return Err(error.into()),
            }
        }
        let mut canonical = fs::canonicalize(existing)?;
        validate_ancestor_chain(&canonical)?;
        let mut fd = File::from(open_absolute_directory(&canonical)?);
        for name in missing.into_iter().rev() {
            reject_unsafe_component(&name)?;
            mkdirat(&fd, &name, PRIVATE_DIR_MODE)?;
            canonical.push(&name);
            let next = open_created_directory(&fd, &name, &canonical)?;
            fd = next;
        }
        Ok((fd, canonical))
    }

    fn validate_ancestor_chain(path: &Path) -> Result<()> {
        for ancestor in path.ancestors() {
            let fd = File::from(open_absolute_directory(ancestor)?);
            secure_directory(&fd, ancestor, true, false)?;
        }
        Ok(())
    }

    fn open_created_directory(parent: &File, name: &OsStr, path: &Path) -> Result<File> {
        let result = (|| -> Result<File> {
            let file = File::from(open_directory(parent, name)?);
            fchmod(&file, PRIVATE_DIR_MODE)?;
            secure_directory(&file, path, false, true)?;
            Ok(file)
        })();
        if result.is_err() {
            let _ = unlinkat(parent, name, AtFlags::REMOVEDIR);
        }
        result
    }

    fn open_absolute_directory(path: &Path) -> Result<OwnedFd> {
        Ok(openat(
            rustix::fs::CWD,
            path,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC | OFlags::NOFOLLOW,
            Mode::empty(),
        )?)
    }

    fn open_directory(parent: &File, name: &OsStr) -> Result<OwnedFd> {
        Ok(openat(
            parent,
            name,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC | OFlags::NOFOLLOW,
            Mode::empty(),
        )?)
    }

    fn secure_directory(
        file: &File,
        path: &Path,
        allow_root: bool,
        require_private_mode: bool,
    ) -> Result<()> {
        let stat = fstat(file)?;
        if FileType::from_raw_mode(stat.st_mode) != FileType::Directory {
            color_eyre::eyre::bail!("session parent is not a directory: {}", path.display())
        }
        let euid = geteuid().as_raw();
        if stat.st_uid != euid && !(allow_root && stat.st_uid == 0) {
            color_eyre::eyre::bail!("session parent has an untrusted owner: {}", path.display())
        }
        if stat.st_mode & 0o022 != 0 {
            color_eyre::eyre::bail!("session parent is group/world writable: {}", path.display())
        }
        if require_private_mode && stat.st_mode & 0o777 != 0o700 {
            color_eyre::eyre::bail!(
                "created session directory permissions are not 0700: {}",
                path.display()
            )
        }
        verify_acl_and_identity(file, path)?;
        Ok(())
    }

    fn secure_regular_file(file: &File, path: &Path) -> Result<()> {
        let stat = fstat(file)?;
        if FileType::from_raw_mode(stat.st_mode) != FileType::RegularFile {
            color_eyre::eyre::bail!("session path is not a regular file: {}", path.display())
        }
        if stat.st_uid != geteuid().as_raw() {
            color_eyre::eyre::bail!("session file has an untrusted owner: {}", path.display())
        }
        fchmod(file, PRIVATE_FILE_MODE)?;
        let secured = fstat(file)?;
        if secured.st_mode & 0o777 != 0o600 {
            color_eyre::eyre::bail!("session file permissions are not 0600: {}", path.display())
        }
        verify_acl_and_identity(file, path)?;
        Ok(())
    }

    fn verify_acl_and_identity(file: &File, path: &Path) -> Result<()> {
        let before = fstat(file)?;
        let entries = getfacl(path, None)?;
        #[cfg(target_os = "linux")]
        let unsafe_acl = entries
            .iter()
            .any(|entry| !entry.name.is_empty() || !entry.flags.is_empty());
        #[cfg(target_os = "macos")]
        let unsafe_acl = !entries.is_empty();
        if unsafe_acl {
            color_eyre::eyre::bail!("session path has an unsupported ACL: {}", path.display())
        }
        let after = statat(rustix::fs::CWD, path, AtFlags::SYMLINK_NOFOLLOW)?;
        if !same_identity(&before, &after) {
            color_eyre::eyre::bail!(
                "session path identity changed during ACL validation: {}",
                path.display()
            )
        }
        Ok(())
    }

    fn same_identity(left: &rustix::fs::Stat, right: &rustix::fs::Stat) -> bool {
        left.st_dev == right.st_dev && left.st_ino == right.st_ino
    }

    fn reject_unsafe_component(name: &OsStr) -> Result<()> {
        let path = Path::new(name);
        if path.components().count() != 1
            || !matches!(path.components().next(), Some(Component::Normal(_)))
        {
            color_eyre::eyre::bail!("unsafe session path component")
        }
        Ok(())
    }

    pub fn secure_private_directory(path: &Path) -> Result<()> {
        let file = File::from(open_absolute_directory(path)?);
        fchmod(&file, PRIVATE_DIR_MODE)?;
        secure_directory(&file, path, false, true)
    }

    pub fn cleanup_private_download(temp_dir: &Path, temp_file: &Path) {
        let Some(dir_name) = temp_dir.file_name() else {
            return;
        };
        let Some(parent_path) = temp_dir.parent() else {
            return;
        };
        let Ok(parent) = open_absolute_directory(parent_path).map(File::from) else {
            return;
        };
        if secure_directory(&parent, parent_path, false, true).is_err() {
            return;
        }
        let Ok(dir) = open_directory(&parent, dir_name).map(File::from) else {
            return;
        };
        if secure_directory(&dir, temp_dir, false, true).is_err() {
            return;
        }
        if let Some(file_name) = temp_file.file_name() {
            let _ = unlinkat(&dir, file_name, AtFlags::empty());
            let _ = unlinkat(&dir, file_name, AtFlags::REMOVEDIR);
        }
        let _ = unlinkat(&parent, dir_name, AtFlags::REMOVEDIR);
    }

    pub fn open_private_file(path: &Path) -> Result<File> {
        let file = File::from(openat(
            rustix::fs::CWD,
            path,
            OFlags::RDWR | OFlags::CLOEXEC | OFlags::NOFOLLOW,
            Mode::empty(),
        )?);
        secure_regular_file(&file, path)?;
        Ok(file)
    }

    pub fn secure_private_file(path: &Path) -> Result<()> {
        open_private_file(path).map(drop)
    }

    pub fn verify_private_file_identity(file: &File, path: &Path) -> Result<()> {
        let opened = fstat(file)?;
        let named = statat(rustix::fs::CWD, path, AtFlags::SYMLINK_NOFOLLOW)?;
        if !same_identity(&opened, &named) {
            color_eyre::eyre::bail!("private file identity changed: {}", path.display())
        }
        verify_acl_and_identity(file, path)
    }

    pub use SecureSessionFile as PlatformSecureSessionFile;

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};
        use std::time::{SystemTime, UNIX_EPOCH};

        fn private_test_dir(label: &str) -> PathBuf {
            let unique = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = PathBuf::from(std::env::var_os("HOME").unwrap()).join(format!(
                ".dumbgram-session-{label}-{}-{unique}",
                std::process::id()
            ));
            fs::create_dir(&path).unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
            path
        }

        #[test]
        fn creates_loads_and_atomically_saves_private_session() {
            let root = private_test_dir("lifecycle");
            let path = root.join("nested/session.dat");
            let (storage, session) = SecureSessionFile::open(&path).unwrap();
            assert_eq!(
                fs::metadata(path.parent().unwrap()).unwrap().mode() & 0o777,
                0o700
            );
            assert_eq!(fs::metadata(&path).unwrap().mode() & 0o777, 0o600);
            storage.save(&session).unwrap();
            let saved = fs::read(&path).unwrap();
            Session::load(&saved).unwrap();
            let (_, loaded) = SecureSessionFile::open(&path).unwrap();
            assert_eq!(loaded.save(), session.save());
            fs::remove_dir_all(root).unwrap();
        }

        #[test]
        fn rejects_symlink_and_nonregular_session_leaf() {
            let root = private_test_dir("leaf-types");
            let target = root.join("target");
            fs::write(&target, Session::new().save()).unwrap();
            let link = root.join("session.dat");
            symlink(&target, &link).unwrap();
            assert!(SecureSessionFile::open(&link).is_err());
            fs::remove_file(&link).unwrap();
            fs::create_dir(&link).unwrap();
            assert!(SecureSessionFile::open(&link).is_err());
            fs::remove_dir_all(root).unwrap();
        }

        #[test]
        fn canonical_parent_binding_ignores_later_lexical_alias_replacement() {
            let root = private_test_dir("alias");
            let trusted = root.join("trusted");
            let replacement = root.join("replacement");
            fs::create_dir(&trusted).unwrap();
            fs::create_dir(&replacement).unwrap();
            fs::set_permissions(&trusted, fs::Permissions::from_mode(0o700)).unwrap();
            fs::set_permissions(&replacement, fs::Permissions::from_mode(0o700)).unwrap();
            let alias = root.join("alias");
            symlink(&trusted, &alias).unwrap();
            let path = alias.join("session.dat");
            let (storage, session) = SecureSessionFile::open(&path).unwrap();
            fs::remove_file(&alias).unwrap();
            symlink(&replacement, &alias).unwrap();
            storage.save(&session).unwrap();
            assert!(trusted.join("session.dat").exists());
            assert!(!replacement.join("session.dat").exists());
            fs::remove_dir_all(root).unwrap();
        }

        #[test]
        fn rejects_writable_parent_and_named_acl() {
            let root = private_test_dir("permissions");
            fs::set_permissions(&root, fs::Permissions::from_mode(0o777)).unwrap();
            assert!(SecureSessionFile::open(&root.join("session.dat")).is_err());
            fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();

            #[cfg(target_os = "linux")]
            {
                use exacl::{AclEntry, Perm, setfacl};
                let entries = vec![
                    AclEntry::allow_user("", Perm::READ | Perm::WRITE | Perm::EXECUTE, None),
                    AclEntry::allow_user("65534", Perm::WRITE | Perm::EXECUTE, None),
                    AclEntry::allow_group("", Perm::empty(), None),
                    AclEntry::allow_mask(Perm::WRITE | Perm::EXECUTE, None),
                    AclEntry::allow_other(Perm::empty(), None),
                ];
                setfacl(&[&root], &entries, None).unwrap();
                assert!(SecureSessionFile::open(&root.join("session.dat")).is_err());
            }
            fs::remove_dir_all(root).unwrap();
        }

        #[test]
        fn failed_stage_never_publishes_malformed_bytes_and_clean_retry_succeeds() {
            let root = private_test_dir("stage-failure");
            let path = root.join("session.dat");
            let (storage, session) = SecureSessionFile::open(&path).unwrap();
            fs::remove_file(&path).unwrap();
            for failure in [
                StageFailure::AfterWrite,
                StageFailure::AfterSync,
                StageFailure::BeforeVerify,
            ] {
                let error = storage
                    .stage_with_failure(&session.save(), Some(failure))
                    .err()
                    .expect("injected stage failure should be preserved");
                assert!(error.to_string().contains("injected stage failure"));
                assert!(!path.exists());
                assert!(fs::read_dir(&root).unwrap().all(|entry| {
                    !entry
                        .unwrap()
                        .file_name()
                        .to_string_lossy()
                        .starts_with(".dumbgram-session-")
                }));
            }
            let (_storage, loaded) = SecureSessionFile::open(&path).unwrap();
            assert_eq!(loaded.save(), Session::new().save());
            fs::remove_dir_all(root).unwrap();
        }

        #[test]
        fn failed_save_stage_preserves_the_old_parseable_session() {
            let root = private_test_dir("save-failure");
            let path = root.join("session.dat");
            let (storage, session) = SecureSessionFile::open(&path).unwrap();
            let old = fs::read(&path).unwrap();
            assert!(
                storage
                    .stage_with_failure(&session.save(), Some(StageFailure::AfterSync))
                    .is_err()
            );
            assert_eq!(fs::read(&path).unwrap(), old);
            Session::load(&old).unwrap();
            fs::remove_dir_all(root).unwrap();
        }

        #[cfg(target_os = "linux")]
        #[test]
        fn rejects_named_acl_on_existing_mode_0600_session_leaf() {
            use exacl::{AclEntry, Perm, setfacl};
            let root = private_test_dir("leaf-acl");
            let path = root.join("session.dat");
            let _ = SecureSessionFile::open(&path).unwrap();
            let entries = vec![
                AclEntry::allow_user("", Perm::READ | Perm::WRITE, None),
                AclEntry::allow_user("65534", Perm::READ, None),
                AclEntry::allow_group("", Perm::empty(), None),
                AclEntry::allow_mask(Perm::READ, None),
                AclEntry::allow_other(Perm::empty(), None),
            ];
            setfacl(&[&path], &entries, None).unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
            assert!(SecureSessionFile::open(&path).is_err());
            fs::remove_dir_all(root).unwrap();
        }

        #[cfg(target_os = "macos")]
        #[test]
        fn rejects_extended_acl_on_existing_mode_0600_session_leaf() {
            use exacl::{AclEntry, Perm, setfacl};
            let root = private_test_dir("leaf-acl");
            let path = root.join("session.dat");
            let _ = SecureSessionFile::open(&path).unwrap();
            setfacl(
                &[&path],
                &[AclEntry::allow_user("0", Perm::READ, None)],
                None,
            )
            .unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
            assert!(SecureSessionFile::open(&path).is_err());
            fs::remove_dir_all(root).unwrap();
        }

        #[test]
        fn published_identity_matches_open_stage_identity() {
            let root = private_test_dir("identity");
            let path = root.join("session.dat");
            let (_storage, _) = SecureSessionFile::open(&path).unwrap();
            let metadata = fs::metadata(&path).unwrap();
            assert_ne!(metadata.ino(), 0);
            assert_eq!(metadata.uid(), geteuid().as_raw());
            fs::remove_dir_all(root).unwrap();
        }
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
pub use unix::{
    PlatformSecureSessionFile as SecureSessionFile, cleanup_private_download, open_private_file,
    secure_private_directory, secure_private_file, verify_private_file_identity,
};

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub struct SecureSessionFile;

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub fn secure_private_directory(_path: &Path) -> Result<()> {
    color_eyre::eyre::bail!("private Telegram cache storage is supported only on Linux and macOS")
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub fn cleanup_private_download(_temp_dir: &Path, _temp_file: &Path) {}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub fn open_private_file(_path: &Path) -> Result<std::fs::File> {
    color_eyre::eyre::bail!("private Telegram cache storage is supported only on Linux and macOS")
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub fn secure_private_file(_path: &Path) -> Result<()> {
    color_eyre::eyre::bail!("private Telegram cache storage is supported only on Linux and macOS")
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub fn verify_private_file_identity(_file: &std::fs::File, _path: &Path) -> Result<()> {
    color_eyre::eyre::bail!("private Telegram cache storage is supported only on Linux and macOS")
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
impl SecureSessionFile {
    pub fn open(_path: &Path) -> Result<(Self, Session)> {
        color_eyre::eyre::bail!("real Telegram sessions are supported only on Linux and macOS")
    }

    pub fn canonical_path(&self) -> std::path::PathBuf {
        unreachable!("unsupported secure-session storage has no canonical path")
    }

    pub fn save(&self, _session: &Session) -> Result<()> {
        color_eyre::eyre::bail!("real Telegram sessions are supported only on Linux and macOS")
    }
}
