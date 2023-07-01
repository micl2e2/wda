// Copyright (C) 2023 Michael Lee <imichael2e2@proton.me OR ...@gmail.com>
//
// Licensed under the MIT License <LICENSE-MIT or
// https://opensource.org/license/mit> or the GNU General Public License,
// Version 3.0 or any later version <LICENSE-GPL or
// https://www.gnu.org/licenses/gpl-3.0.txt>, at your option.
//
// This file may not be copied, modified, or distributed except except in
// compliance with either of the licenses.
//

use crate::error::Result;
use crate::error::WdaError;

use std::collections::HashMap;
use std::ffi::OsString;
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::process::Stdio;

use std::fs;
use std::fs::create_dir_all;

// lock //

#[cfg(target_family = "unix")]
mod lock {
    use std::fs::File;
    use std::os::fd::AsRawFd;

    fn flock(file: &File, flag: libc::c_int) -> std::io::Result<()> {
        let ret = unsafe { libc::flock(file.as_raw_fd(), flag) };
        if ret < 0 {
            Err(std::io::Error::last_os_error())
        } else {
            Ok(())
        }
    }

    pub fn acquire(lock: &File) -> Result<(), u8> {
        flock(&lock, libc::LOCK_EX).unwrap();

        Ok(())
    }

    pub fn release(lock: &File) -> Result<(), u8> {
        flock(&lock, libc::LOCK_UN).unwrap();

        Ok(())
    }
}

#[cfg(target_family = "windows")]
mod lock {
    use std::fs::File;
    use std::os::windows::io::AsRawHandle;

    use winapi::um::errhandlingapi::GetLastError;
    use winapi::um::fileapi::LockFileEx;
    use winapi::um::fileapi::UnlockFileEx;
    use winapi::um::minwinbase::LOCKFILE_EXCLUSIVE_LOCK;
    use winapi::um::minwinbase::LOCKFILE_FAIL_IMMEDIATELY;
    use winapi::um::minwinbase::OVERLAPPED;

    const LOCKED_LEN: u32 = 2; // sizeof u16

    pub fn acquire(lock: &File) -> Result<(), u8> {
        let mut ol = OVERLAPPED::default();
        unsafe {
            let is_success = LockFileEx(
                lock.as_raw_handle(),
                LOCKFILE_EXCLUSIVE_LOCK,
                0,
                LOCKED_LEN,
                0,
                &mut ol,
            );
            if is_success == 0 {
                panic!("lock fail: lasterror {}", GetLastError());
            }
        }

        Ok(())
    }

    pub fn release(lock: &File) -> Result<(), u8> {
        let mut ol = OVERLAPPED::default();
        unsafe {
            let is_success = UnlockFileEx(lock.as_raw_handle(), 0, LOCKED_LEN, 0, &mut ol);
            if is_success == 0 {
                panic!("unlock fail: lasterror {}", GetLastError());
            }
        }
        Ok(())
    }
}

pub(crate) use lock::acquire as lock_acquire;
pub(crate) use lock::release as lock_release;

//

#[derive(Debug, Clone, Copy)]
pub(crate) enum BrowserFamily {
    Firefox,
    Chromium,
}

impl BrowserFamily {
    fn profile_prefix(&self) -> &'static str {
        match self {
            BrowserFamily::Firefox => "fox",
            BrowserFamily::Chromium => "chr",
        }
    }
}

// WdaWorkingdir //

#[derive(Debug)]
pub(super) struct WdaWorkingDir {
    home_pbuf: PathBuf,
    data_root: &'static str,
    sver: &'static str, // structure version
    rend_dir: &'static str,
    lock_dir: &'static str,
    log_dir: &'static str,
    cache_dir: &'static str,
    bprof_dir: &'static str,
}

impl WdaWorkingDir {
    pub(crate) fn existing_lock(&self, lock_name: &str) -> Result<File> {
        if let Ok(flag) = Path::new(&self.lock_file_pbuf(lock_name)).try_exists() {
            if !flag {
                return Err(WdaError::WdaDataNotFound);
            }
        } else {
            return Err(WdaError::Buggy);
        }

        Ok(OpenOptions::new()
            .read(true)
            .write(true)
            .open(
                &self
                    .home_pbuf
                    .join(self.data_root)
                    .join(self.sver)
                    .join(self.lock_dir)
                    .join(lock_name),
            )
            .unwrap())
    }

    pub(crate) fn zero_log(&self, log_name: &str) -> File {
        OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(
                &self
                    .home_pbuf
                    .join(self.data_root)
                    .join(self.sver)
                    .join(self.log_dir)
                    .join(log_name),
            )
            .expect("failed to open zero writable file")
    }

    pub(crate) fn rend_as_command(&self, rend_id: &str) -> Command {
        Command::new(self.rend_file_pbuf(rend_id))
    }

    pub(crate) fn download(&self, rend_id: &str, dl_proxy: Option<&str>) -> Result<()> {
        // if exists, we are done
        if let Ok(flag) = Path::new(&self.rend_file_pbuf(rend_id)).try_exists() {
            if flag {
                return Ok(());
            }
        } else {
            return Err(WdaError::Buggy);
        }

        check_dl_tools()?;

        let mut map = HashMap::<&str, Vec<&str>>::new();

        map.insert(
        "geckodriver-v0.32.2-linux64",
        vec!["https://github.com/mozilla/geckodriver/releases/download/v0.32.2/geckodriver-v0.32.2-linux64.tar.gz","geckodriver-v0.32.2-linux64.tar.gz","geckodriver"],
    );
        map.insert(
        "geckodriver-v0.30.0-linux64",
        vec!["https://github.com/mozilla/geckodriver/releases/download/v0.30.0/geckodriver-v0.30.0-linux64.tar.gz","geckodriver-v0.30.0-linux64.tar.gz","geckodriver"],
    );
        map.insert(
	    "geckodriver-v0.30.0-win64.exe",
        vec![
            "https://github.com/mozilla/geckodriver/releases/download/v0.30.0/geckodriver-v0.30.0-win64.zip",
            "geckodriver-v0.30.0-win64.zip",
            "geckodriver.exe",
        ],
    );
        map.insert(
        "geckodriver-v0.30.0-macos",
        vec!["https://github.com/mozilla/geckodriver/releases/download/v0.30.0/geckodriver-v0.30.0-macos.tar.gz","geckodriver-v0.30.0-macos.tar.gz","geckodriver"],
    );
        map.insert(
            "chromedriver-v112-linux64",
            vec![
            "https://chromedriver.storage.googleapis.com/112.0.5615.49/chromedriver_linux64.zip"
                ,
            "chromedriver-v112-linux64.zip",
            "chromedriver",
        ],
        );
        map.insert(
            "chromedriver-v112-win32.exe",
            vec![
                "https://chromedriver.storage.googleapis.com/112.0.5615.49/chromedriver_win32.zip",
                "chromedriver-v112-win32.zip",
                "chromedriver.exe",
            ],
        );
        map.insert(
            "chromedriver-v112-mac64",
            vec![
                "https://chromedriver.storage.googleapis.com/112.0.5615.49/chromedriver_mac64.zip",
                "chromedriver-v112-mac64.zip",
                "chromedriver",
            ],
        );

        let vals = map.get(rend_id);
        if vals.is_none() {
            return Err(WdaError::RendNotSupported);
        }
        let vals = vals.unwrap();
        let url = vals[0];
        let tarfile = vals[1];
        let rend_file_in_tar = vals[2];

        dbgmsg!("downloading '{}'...", rend_id);

        let wdadata_lock = self.new_lock_file("wdadata");

        // ---
        lock_acquire(&wdadata_lock).unwrap();

        #[allow(unused_assignments)]
        let mut operation_failed = true;

        // Download //
        let mut curl_args = vec!["--location"];
        if dl_proxy.is_some() {
            curl_args.push("--socks5");
            curl_args.push(dl_proxy.unwrap());
        }
        curl_args.push(url);
        curl_args.push("--output");
        let dest_tar = &self
            .cache_file_pbuf(tarfile)
            .into_os_string()
            .into_string()
            .unwrap();
        curl_args.push(&dest_tar);
        let status = Command::new("curl")
            .args(curl_args)
            .stdout(self.zero_log(&format!("fetch-out.{}.log", rend_id)))
            .stderr(self.zero_log(&format!("fetch-err.{}.log", rend_id)))
            .status()
            .expect("failed to download ");

        // Extract //
        if !status.success() {
            let excode = status.code().unwrap();
            return Err(WdaError::FetchWebDriver(excode));
        }

        let _status = if tarfile.contains(".tar.gz") {
            Command::new("tar")
                .args(["--extract", "--file", &dest_tar, rend_file_in_tar])
                .stdout(self.zero_log(&format!("tar-out.{}.log", rend_id)))
                .stderr(self.zero_log(&format!("tar-err.{}.log", rend_id)))
                .status()
                .expect("failed to extract")
        } else if tarfile.contains(".zip") {
            Command::new("unzip")
                .args([&dest_tar, rend_file_in_tar])
                .stdout(self.zero_log(&format!("extract-out.{}.log", rend_id)))
                .stderr(self.zero_log(&format!("extract-err.{}.log", rend_id)))
                .status()
                .expect("failed to extract")
        } else {
            panic!("unsupported archive")
        };

        // Permit //

        // permit before rename bc chmod on win cannot apply on any file path
        // with '\', its bug
        let status = Command::new("chmod")
            .args(["+x", rend_file_in_tar])
            .stdout(self.zero_log(&format!("permit-out.{}.log", rend_id)))
            .stderr(self.zero_log(&format!("permit-err.{}.log", rend_id)))
            .status()
            .expect("permit");
        if !status.success() {
            let excode = status.code().unwrap();
            return Err(WdaError::PermitWebDriver(excode));
        }

        // Rename //
        let rend_file_renamed = self
            .rend_file_pbuf(rend_id)
            .into_os_string()
            .into_string()
            .unwrap();
        if !status.success() {
            let excode = status.code().unwrap();
            return Err(WdaError::ExtractWebDriver(excode));
        }
        let status = Command::new("mv")
            .args([rend_file_in_tar, &rend_file_renamed])
            .stdout(self.zero_log(&format!("extract-out.{}.log", rend_id)))
            .stderr(self.zero_log(&format!("extract-err.{}.log", rend_id)))
            .status()
            .expect("extract");
        if !status.success() {
            let excode = status.code().unwrap();
            return Err(WdaError::PlaceWebDriver(excode));
        }

        // done //
        dbgmsg!("downloading '{}'...done", rend_id);
        operation_failed = false;

        lock_release(&wdadata_lock).unwrap();
        // ---

        if operation_failed {
            Err(WdaError::Buggy)
        } else {
            Ok(())
        }
    }

    ///
    /// Create a fresh browser profile(directory), and return it if successful.
    pub(crate) fn fresh_bprof(&self, bfam: BrowserFamily) -> Result<PathBuf> {
        // this fn has a remove_dir_all
        let bprof_lock = self.existing_lock("bprof")?;

        lock_acquire(&bprof_lock).expect("bug");

        let prefix = bfam.profile_prefix();

        let mut dirs = Vec::<String>::new();

        match Path::new(&self.bprof_dir()).try_exists() {
            Ok(flag) => {
                if !flag {
                    return Err(WdaError::BrowserProfileRootNotFound);
                }
            }
            Err(_) => {
                return Err(WdaError::Buggy);
            }
        }

        for may_entry in std::fs::read_dir(self.bprof_dir()).expect("bug") {
            if let Ok(entry) = may_entry {
                let fname = entry.file_name().into_string().expect("bug");
                if &fname[0..3] == prefix {
                    dirs.push(fname);
                }
            }
        }

        let new_bpname: String;

        if dirs.len() > 0 {
            dirs.sort();
            let lelem = dirs.last().expect("bug");
            let npart = u16::from_str_radix(&lelem[3..], 10).expect("bug");
            let nnpart = npart + 1;
            new_bpname = if nnpart < 10 {
                format!("{}0{}", prefix, nnpart)
            } else if nnpart < 100 {
                format!("{}{}", prefix, nnpart)
            } else {
                let bp_pbuf = self
                    .home_pbuf
                    .join(self.data_root)
                    .join(self.sver)
                    .join(self.bprof_dir);
                std::fs::remove_dir_all(&bp_pbuf).expect("bug");
                create_dir_all(&bp_pbuf).expect("bug");
                format!("{}00", prefix)
            };
        } else {
            new_bpname = format!("{}00", prefix);
        }

        let pbuf = self
            .home_pbuf
            .join(self.data_root)
            .join(self.sver)
            .join(self.bprof_dir)
            .join(&new_bpname);
        create_dir_all(&pbuf).expect("bug");

        lock_release(&bprof_lock).expect("bug");

        Ok(pbuf)
    }

    ///
    /// Grab the most recently used browser profile(directory). In
    /// cases where there is no existing browser profiles, `None` is returned.
    pub(crate) fn last_bprof(&self, bfam: BrowserFamily) -> Result<Option<PathBuf>> {
        // if prefix.len() != 3 {
        //     return None;
        // }

        match Path::new(&self.bprof_dir()).try_exists() {
            Ok(flag) => {
                if !flag {
                    return Err(WdaError::BrowserProfileRootNotFound);
                }
            }
            Err(_) => {
                return Err(WdaError::Buggy);
            }
        }

        let prefix = bfam.profile_prefix();

        let mut dirs = Vec::<OsString>::new();
        for may_entry in fs::read_dir(self.bprof_dir()).expect("buggy") {
            if let Ok(entry) = may_entry {
                let fname = entry.file_name().into_string().expect("bug");
                if &fname[0..3] == prefix {
                    dirs.push(entry.file_name());
                }
            }
        }
        if dirs.len() > 0 {
            dirs.sort();

            Ok(Some(
                self.home_pbuf
                    .join(self.data_root)
                    .join(self.sver)
                    .join(self.bprof_dir)
                    .join(&dirs.last().expect("bug")),
            ))
        } else {
            Ok(None)
        }
    }

    // private //

    fn new_droot_lock(&self, lock_name: &str) -> File {
        OpenOptions::new()
            .create(true)
            .write(true)
            .open(&self.home_pbuf.join(lock_name))
            .expect(&format!("failed to open new lock file {}", lock_name))
    }

    fn new_lock_file(&self, lock_name: &str) -> File {
        OpenOptions::new()
            .create(true)
            .write(true)
            .open(
                &self
                    .home_pbuf
                    .join(self.data_root)
                    .join(self.sver)
                    .join(self.lock_dir)
                    .join(lock_name),
            )
            .expect(&format!("failed to open new lock file {}", lock_name))
    }

    fn cache_file_pbuf(&self, fname: &str) -> PathBuf {
        self.home_pbuf
            .join(self.data_root)
            .join(self.sver)
            .join(self.cache_dir)
            .join(fname)
    }

    fn rend_file_pbuf(&self, fname: &str) -> PathBuf {
        self.home_pbuf
            .join(self.data_root)
            .join(self.sver)
            .join(self.rend_dir)
            .join(fname)
    }

    fn lock_file_pbuf(&self, fname: &str) -> PathBuf {
        self.home_pbuf
            .join(self.data_root)
            .join(self.sver)
            .join(self.lock_dir)
            .join(fname)
    }

    fn bprof_dir(&self) -> PathBuf {
        self.home_pbuf
            .join(self.data_root)
            .join(self.sver)
            .join(self.bprof_dir)
    }
}

// misc //

fn check_dl_tools() -> Result<()> {
    // curl
    let curl_cmd = Command::new("curl")
        .args(["--help"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();

    match curl_cmd {
        Ok(exstat) => {
            if !exstat.success() {
                dbgmsg!("'curl' is buggy");
                return Err(WdaError::FetchToolBuggy);
            } else {
                dbgmsg!("program `curl` is ready!");
            }
        }
        Err(io_err) => match io_err.kind() {
            std::io::ErrorKind::NotFound => {
                dbgmsg!("'curl' is not found");
                return Err(WdaError::FetchToolNotFound);
            }
            _e => {
                dbgmsg!("{:?}", _e);
            }
        },
    }

    // unzip
    let unzip_cmd = Command::new("unzip")
        .args(["--help"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();

    match unzip_cmd {
        Ok(exstat) => {
            if !exstat.success() {
                dbgmsg!("'unzip' is buggy");
                return Err(WdaError::ExtractToolBuggy);
            } else {
                dbgmsg!("program `unzip` is ready!");
            }
        }
        Err(io_err) => match io_err.kind() {
            std::io::ErrorKind::NotFound => {
                dbgmsg!("'unzip' is not found");
                return Err(WdaError::ExtractToolNotFound);
            }
            _e => {
                dbgmsg!("{:?}", _e);
            }
        },
    }

    // tar
    let tar_cmd = Command::new("tar")
        .args(["--help"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();

    match tar_cmd {
        Ok(exstat) => {
            if !exstat.success() {
                dbgmsg!("'tar' is buggy");
                return Err(WdaError::ExtractToolBuggy);
            } else {
                dbgmsg!("program `tar` is ready!");
            }
        }
        Err(io_err) => match io_err.kind() {
            std::io::ErrorKind::NotFound => {
                dbgmsg!("'tar' is not found");
                return Err(WdaError::ExtractToolNotFound);
            }
            _e => {
                dbgmsg!("{:?}", _e);
            }
        },
    }

    // mv
    let mv_cmd = Command::new("which")
        .args(["mv"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();

    match mv_cmd {
        Ok(exstat) => {
            if !exstat.success() {
                dbgmsg!("`mv` is buggy");
                return Err(WdaError::RenameToolBuggy);
            } else {
                dbgmsg!("program `mv` is ready!");
            }
        }
        Err(io_err) => match io_err.kind() {
            std::io::ErrorKind::NotFound => {
                dbgmsg!("`mv` is not found");
                return Err(WdaError::RenameToolNotFound);
            }
            _e => {
                dbgmsg!("{:?}", _e);
            }
        },
    }

    Ok(())
}

#[cfg(target_family = "unix")]
fn get_home_dir() -> String {
    use std::env;
    for (k, v) in env::vars() {
        if k == "HOME" {
            return v;
        }
    }
    return "".to_owned();
}

#[cfg(target_family = "windows")]
fn get_home_dir() -> String {
    use std::env;
    for (k, v) in env::vars() {
        if k == "HOME" {
            return v;
        }
    }
    return "".to_owned();
}

// // a cannot-fail operation
// fn create_wda_workdirs() -> WdaWorkingDir {
//     let home_dir = get_home_dir();
//     let data_root = ".wdadata";
//     let sver = "v1"; // currently v1 structure in use
//     let rend_dir = "rend";
//     let lock_dir = "lock";
//     let log_dir = "log";
//     let cache_dir = "cache";
//     let bprof_dir = "bprofile";

//     // manually delete data_root to reset all setting

//     let home_pbuf = PathBuf::new().join(&home_dir);

//     create_dir_all(home_pbuf.join(data_root).join(sver)).unwrap();
//     create_dir_all(home_pbuf.join(data_root).join(sver).join(rend_dir)).unwrap();
//     create_dir_all(home_pbuf.join(data_root).join(sver).join(lock_dir)).unwrap();
//     create_dir_all(home_pbuf.join(data_root).join(sver).join(cache_dir)).unwrap();
//     create_dir_all(home_pbuf.join(data_root).join(sver).join(log_dir)).unwrap();
//     create_dir_all(home_pbuf.join(data_root).join(sver).join(bprof_dir)).unwrap();

//     WdaWorkingDir {
//         home_pbuf,
//         data_root,
//         sver,
//         rend_dir,
//         lock_dir,
//         log_dir,
//         cache_dir,
//         bprof_dir,
//     }
// }

// a cannot-fail operation
fn create_wda_workdirs2(
    reset: bool,
    predef_home: Option<&'static str>,
    predef_root: Option<&'static str>,
) -> WdaWorkingDir {
    let real_home = get_home_dir();

    let home_dir = if predef_home.is_some() {
        predef_home.unwrap()
    } else {
        &real_home
    };

    let data_root = if predef_root.is_some() {
        predef_root.unwrap()
    } else {
        ".wda"
    };
    let sver = "v1"; // currently v1 structure in use
    let rend_dir = "rend";
    let lock_dir = "lock";
    let log_dir = "log";
    let cache_dir = "cache";
    let bprof_dir = "bprof";

    // manually delete data_root to reset all setting

    let home_pbuf = PathBuf::new().join(&home_dir);

    if reset {
        if let Err(_) = fs::remove_dir_all(
            /* double check!!! */
            home_pbuf.join(data_root), /* double check!!! */
        ) {}
    }

    // create itself and its all subs
    fs::create_dir_all(home_pbuf.join(data_root).join(sver)).unwrap();
    fs::create_dir_all(home_pbuf.join(data_root).join(sver).join(rend_dir)).unwrap();
    fs::create_dir_all(home_pbuf.join(data_root).join(sver).join(lock_dir)).unwrap();
    fs::create_dir_all(home_pbuf.join(data_root).join(sver).join(cache_dir)).unwrap();
    fs::create_dir_all(home_pbuf.join(data_root).join(sver).join(log_dir)).unwrap();
    fs::create_dir_all(home_pbuf.join(data_root).join(sver).join(bprof_dir)).unwrap();

    WdaWorkingDir {
        home_pbuf,
        data_root,
        sver,
        rend_dir,
        lock_dir,
        log_dir,
        cache_dir,
        bprof_dir,
    }
}

// ///
// /// Prepare essential data for Wda instances.
// ///
// /// Typically it consists of following steps:
// ///
// /// 1. All well-organized directories are in place.
// /// 2. Ensure that plock file is not corrupt.
// /// 3. Ensure a reasonable number of Wda instances do not interfere with
// /// each other.
// ///
// /// After prepared, Wda instances can be readily created, and be
// /// multi-threadly safely used.
// pub(crate) fn Xprepare_wdir() -> Result<WdaWorkingDir> {
//     let wda_wdir = create_wda_workdirs();

//     let droot_lock = wda_wdir.new_lock_file("wdadata");

//     // ---
//     lock_acquire(&droot_lock).unwrap();

//     // for testing purpose, simulate massive tasks on exclusive occupation
//     // sleep(Duration::from_secs(1));

//     let mut plock;

//     plock = "gecrend";
//     ensure_valid_plock(&wda_wdir, plock, 4444)?;
//     plock = "chrrend";
//     ensure_valid_plock(&wda_wdir, plock, 9515)?;

//     dbgmsg!("preparing wda essential data...done");

//     lock_release(&droot_lock).unwrap();
//     // ---

//     Ok(wda_wdir)
// }

///
/// Prepare essential data for Wda instances.
///
/// Typically it consists of following steps:
///
/// 1. All well-organized directories are in place.
/// 2. Ensure that plock file is not corrupt.
/// 3. Ensure a reasonable number of Wda instances do not interfere with
/// each other.
///
/// After prepared, Wda instances can be readily created, and be
/// multi-threadly safely used.
///
/// if `reset` is `true`, work dir would be removed forcibly before prepare. Use with caution!
pub(crate) fn prepare_wdir(
    reset: bool,
    predef_home: Option<&'static str>,
    predef_root: Option<&'static str>,
) -> Result<WdaWorkingDir> {
    let wda_wdir = create_wda_workdirs2(reset, predef_home, predef_root);

    let droot_lock = wda_wdir.new_droot_lock(".wda.lock");

    // ---
    lock_acquire(&droot_lock).unwrap();

    // for testing purpose, simulate massive tasks on exclusive occupation
    // sleep(Duration::from_secs(1));

    let mut plock;

    plock = "gecrend";
    ensure_valid_plock(&wda_wdir, plock, 4445)?;
    plock = "chrrend";
    ensure_valid_plock(&wda_wdir, plock, 9516)?;

    // bprof lock
    let _ = wda_wdir.new_lock_file("bprof");

    lock_release(&droot_lock).unwrap();
    // ---

    Ok(wda_wdir)
}

fn ensure_valid_plock(wdir: &WdaWorkingDir, plock: &str, default: u16) -> Result<()> {
    match wdir.existing_lock(plock) {
        Ok(mut f) => {
            let mut buf = [0u8; 2];
            if let Err(_e) = f.read_exact(&mut buf) {
                dbgg!(_e);
                return Err(WdaError::WdaDataNotFound);
            }
            let nport = u16::from_le_bytes(buf);
            dbgg!(nport);
            if nport < default {
                Err(WdaError::PlockDataCorrupt)
            } else {
                Ok(())
            }
        }
        Err(e) => match e {
            WdaError::WdaDataNotFound => {
                let mut f = wdir.new_lock_file(plock);
                let port = default.to_le_bytes();
                f.write_all(&port).unwrap();
                Ok(())
            }
            _e => {
                dbgg!(_e);
                Err(WdaError::Buggy)
            }
        },
    }
}

// unit tests //

// note: these are not strictly unit ones, but integrated ones, placing them
//       here is bc this is crate-public module

#[cfg(test)]
mod utst_singl_thread {
    use super::*;

    #[allow(non_snake_case)]
    fn _0(
        HOME_DIR: &'static str,
        DATAROOT_DIR: &'static str,
        TESTING_PREFIX: &'static str,
        BFAM: BrowserFamily,
    ) {
        let wdir = prepare_wdir(
            true, /* means delete all */
            Some(HOME_DIR),
            Some(DATAROOT_DIR),
        )
        .expect("bug");

        // is empty dir
        assert_eq!(wdir.last_bprof(BFAM).expect("bug"), None);

        // make 100 fresh ones
        for i in 0..100 {
            let pbuf = wdir.fresh_bprof(BFAM).expect("bug");
            let expected_pbuf = if i < 10 {
                PathBuf::new()
                    .join(wdir.bprof_dir())
                    .join(format!("{}0{}", TESTING_PREFIX, i))
            } else {
                PathBuf::new()
                    .join(wdir.bprof_dir())
                    .join(format!("{}{}", TESTING_PREFIX, i))
            };

            // only predictable on single-thread env
            assert_eq!(pbuf, expected_pbuf);
            assert!(wdir.last_bprof(BFAM).expect("bug").is_some());
            assert_eq!(wdir.last_bprof(BFAM).expect("bug").unwrap(), expected_pbuf);
        }
        assert_eq!(
            wdir.last_bprof(BFAM).expect("bug"),
            Some(
                PathBuf::new()
                    .join(wdir.bprof_dir())
                    .join(format!("{}99", TESTING_PREFIX))
            )
        );

        // make one more fresh, cause previous 99 removed
        for _ in 0..1 {
            let pbuf = wdir.fresh_bprof(BFAM).expect("bug");
            assert_eq!(
                pbuf,
                PathBuf::new()
                    .join(wdir.bprof_dir())
                    .join(format!("{}00", TESTING_PREFIX))
            );
        }
        assert_eq!(
            wdir.last_bprof(BFAM).expect("bug"),
            Some(
                PathBuf::new()
                    .join(wdir.bprof_dir())
                    .join(format!("{}00", TESTING_PREFIX))
            )
        );
    }

    #[test]
    fn _1() {
        _0("/tmp", ".tstwda1", "fox", BrowserFamily::Firefox);
    }

    #[test]
    fn _2() {
        _0("/tmp", ".tstwda2", "chr", BrowserFamily::Chromium);
    }
}

#[cfg(test)]
mod utst_multi_thread {
    use super::*;

    #[allow(non_snake_case)]
    fn _0(HOME_DIR: &'static str, DATAROOT_DIR: &'static str, BFAM: BrowserFamily, TIMES: usize) {
        let wdir = prepare_wdir(false, Some(HOME_DIR), Some(DATAROOT_DIR)).expect("bug");

        // could be none or some
        let may_last_profile = wdir.last_bprof(BFAM);
        assert!(may_last_profile.is_ok());
        let last_profile = may_last_profile.unwrap();
        assert!(last_profile.is_some() || last_profile.is_none());

        // make N fresh ones
        for _ in 0..TIMES {
            let _ = wdir.fresh_bprof(BFAM).expect("bug");
        }
    }

    #[test]
    fn _1() {
        // test fresh_bprof's multi-threaded safety

        let wdir = prepare_wdir(
            true, /* means delete all */
            Some("/tmp"),
            Some(".tstwda3"),
        )
        .expect("bug");

        let th1 = std::thread::spawn(|| {
            _0("/tmp", ".tstwda3", BrowserFamily::Firefox, 60);
        });
        let th2 = std::thread::spawn(|| {
            _0("/tmp", ".tstwda3", BrowserFamily::Firefox, 60);
        });

        th1.join().unwrap();
        th2.join().unwrap();

        let last_profile = wdir.last_bprof(BrowserFamily::Firefox).expect("bug");
        assert!(last_profile.is_some());
        assert_eq!(
            last_profile.unwrap(),
            PathBuf::new().join(wdir.bprof_dir()).join("fox19") // 120-100
        );
    }
}