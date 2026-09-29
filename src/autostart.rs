//! Start-on-login: registry Run key on Windows, LaunchAgent on macOS, XDG autostart on Linux.

use std::io;

pub fn is_enabled() -> bool {
    imp::is_enabled()
}

pub fn set(enabled: bool) -> io::Result<()> {
    imp::set(enabled)
}

fn current_exe() -> io::Result<String> {
    Ok(std::env::current_exe()?.to_string_lossy().into_owned())
}

#[cfg(windows)]
mod imp {
    use std::{io, ptr};
    use windows_sys::Win32::{
        Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS},
        System::Registry::{
            HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_SZ, RegDeleteKeyValueW, RegGetValueW,
            RegSetKeyValueW,
        },
    };

    const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
    const VALUE: &str = "proj";

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(Some(0)).collect()
    }

    pub fn is_enabled() -> bool {
        let (key, value) = (wide(RUN_KEY), wide(VALUE));
        let status = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                key.as_ptr(),
                value.as_ptr(),
                RRF_RT_REG_SZ,
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null_mut(),
            )
        };
        status == ERROR_SUCCESS
    }

    pub fn set(enabled: bool) -> io::Result<()> {
        let (key, value) = (wide(RUN_KEY), wide(VALUE));
        let status = if enabled {
            let data = wide(&format!("\"{}\"", super::current_exe()?));
            unsafe {
                RegSetKeyValueW(
                    HKEY_CURRENT_USER,
                    key.as_ptr(),
                    value.as_ptr(),
                    REG_SZ,
                    data.as_ptr().cast(),
                    (data.len() * 2) as u32,
                )
            }
        } else {
            match unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, key.as_ptr(), value.as_ptr()) } {
                ERROR_FILE_NOT_FOUND => ERROR_SUCCESS,
                status => status,
            }
        };
        if status == ERROR_SUCCESS {
            Ok(())
        } else {
            Err(io::Error::from_raw_os_error(status as i32))
        }
    }
}

#[cfg(not(windows))]
mod imp {
    use std::{fs, io, path::PathBuf};

    fn entry_path() -> Option<PathBuf> {
        if cfg!(target_os = "macos") {
            Some(dirs::home_dir()?.join("Library/LaunchAgents/dev.proj.launcher.plist"))
        } else {
            Some(dirs::config_dir()?.join("autostart/proj.desktop"))
        }
    }

    pub fn is_enabled() -> bool {
        entry_path().is_some_and(|p| p.exists())
    }

    pub fn set(enabled: bool) -> io::Result<()> {
        let path = entry_path().ok_or_else(|| io::Error::other("no home directory"))?;
        if !enabled {
            return match fs::remove_file(&path) {
                Err(err) if err.kind() != io::ErrorKind::NotFound => Err(err),
                _ => Ok(()),
            };
        }
        let exe = super::current_exe()?;
        let contents = if cfg!(target_os = "macos") {
            format!(
                r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key><string>dev.proj.launcher</string>
    <key>ProgramArguments</key><array><string>{exe}</string></array>
    <key>RunAtLoad</key><true/>
</dict>
</plist>
"#
            )
        } else {
            format!(
                "[Desktop Entry]\nType=Application\nName=proj\nExec=\"{exe}\"\nX-GNOME-Autostart-enabled=true\n"
            )
        };
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, contents)
    }
}
