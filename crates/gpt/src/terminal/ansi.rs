use std::env;
use std::io::{self, IsTerminal};

pub(crate) const ANSI_GREEN: &str = "\x1b[32m";
pub(super) const ANSI_RESET: &str = "\x1b[0m";
const ANSI_RED: &str = "\x1b[31m";

pub fn format_cli_error(err: &anyhow::Error) -> String {
    format_cli_error_with_color(err, ansi_color_enabled_for_stderr())
}

pub(super) fn format_cli_error_with_color(err: &anyhow::Error, ansi_color: bool) -> String {
    if ansi_color {
        format!("{ANSI_RED}error:{ANSI_RESET} {err:#}")
    } else {
        format!("error: {err:#}")
    }
}

pub(super) fn ansi_green_enabled_for_stdout() -> bool {
    if !io::stdout().is_terminal() {
        return false;
    }
    let env = env::vars().collect::<Vec<_>>();
    ansi_color_supported(&env, windows_stdout_supports_ansi()) && shell_background_is_black(&env)
}

fn ansi_color_enabled_for_stderr() -> bool {
    if !io::stderr().is_terminal() {
        return false;
    }
    ansi_color_supported(
        &env::vars().collect::<Vec<_>>(),
        windows_stderr_supports_ansi(),
    )
}

pub(super) fn ansi_color_supported(
    env: &[(String, String)],
    windows_console_support: Option<bool>,
) -> bool {
    let get = |key: &str| {
        env.iter()
            .find(|(candidate, _)| candidate == key)
            .map(|(_, value)| value.as_str())
    };

    if get("NO_COLOR").is_some() {
        return false;
    }

    if matches!(get("CLICOLOR_FORCE"), Some(value) if value != "0")
        || matches!(get("FORCE_COLOR"), Some(value) if value != "0")
    {
        return true;
    }

    let term = get("TERM").unwrap_or_default();
    if term.eq_ignore_ascii_case("dumb") {
        return false;
    }

    #[cfg(windows)]
    {
        windows_console_support.unwrap_or(false)
            || get("WT_SESSION").is_some()
            || get("ANSICON").is_some()
            || matches!(get("ConEmuANSI"), Some(value) if value.eq_ignore_ascii_case("ON"))
            || (!term.is_empty() && !term.eq_ignore_ascii_case("dumb"))
    }

    #[cfg(not(windows))]
    {
        windows_console_support.unwrap_or(!term.is_empty())
    }
}

pub(super) fn shell_background_is_black(env: &[(String, String)]) -> bool {
    #[cfg(windows)]
    if let Some(is_black) = windows_console_background_is_black() {
        return is_black;
    }

    let colorfgbg = env
        .iter()
        .find(|(candidate, _)| candidate == "COLORFGBG")
        .map(|(_, value)| value.as_str());

    let Some(colorfgbg) = colorfgbg else {
        return false;
    };

    colorfgbg
        .rsplit(';')
        .next()
        .and_then(|value| value.trim().parse::<u8>().ok())
        == Some(0)
}

#[cfg(windows)]
fn windows_stdout_supports_ansi() -> Option<bool> {
    windows_console_supports_ansi(-11)
}

#[cfg(not(windows))]
fn windows_stdout_supports_ansi() -> Option<bool> {
    None
}

#[cfg(windows)]
fn windows_stderr_supports_ansi() -> Option<bool> {
    windows_console_supports_ansi(-12)
}

#[cfg(not(windows))]
fn windows_stderr_supports_ansi() -> Option<bool> {
    None
}

#[cfg(windows)]
fn windows_console_supports_ansi(std_handle: i32) -> Option<bool> {
    const ENABLE_VIRTUAL_TERMINAL_PROCESSING: u32 = 0x0004;

    unsafe extern "system" {
        fn GetStdHandle(n_std_handle: i32) -> *mut std::ffi::c_void;
        fn GetConsoleMode(h_console_handle: *mut std::ffi::c_void, lp_mode: *mut u32) -> i32;
        fn SetConsoleMode(h_console_handle: *mut std::ffi::c_void, dw_mode: u32) -> i32;
    }

    let handle = unsafe { GetStdHandle(std_handle) };
    if handle.is_null() || handle as isize == -1 {
        return None;
    }

    let mut mode = 0u32;
    if unsafe { GetConsoleMode(handle, &mut mode) } == 0 {
        return None;
    }

    if (mode & ENABLE_VIRTUAL_TERMINAL_PROCESSING) != 0 {
        return Some(true);
    }

    let enabled_mode = mode | ENABLE_VIRTUAL_TERMINAL_PROCESSING;
    Some(unsafe { SetConsoleMode(handle, enabled_mode) != 0 })
}

#[cfg(windows)]
fn windows_console_background_is_black() -> Option<bool> {
    const STD_OUTPUT_HANDLE: i32 = -11;

    #[repr(C)]
    struct Coord {
        x: i16,
        y: i16,
    }

    #[repr(C)]
    struct SmallRect {
        left: i16,
        top: i16,
        right: i16,
        bottom: i16,
    }

    #[repr(C)]
    struct ConsoleScreenBufferInfo {
        dw_size: Coord,
        dw_cursor_position: Coord,
        w_attributes: u16,
        sr_window: SmallRect,
        dw_maximum_window_size: Coord,
    }

    unsafe extern "system" {
        fn GetStdHandle(n_std_handle: i32) -> *mut std::ffi::c_void;
        fn GetConsoleScreenBufferInfo(
            h_console_output: *mut std::ffi::c_void,
            lp_console_screen_buffer_info: *mut ConsoleScreenBufferInfo,
        ) -> i32;
    }

    let handle = unsafe { GetStdHandle(STD_OUTPUT_HANDLE) };
    if handle.is_null() || handle as isize == -1 {
        return None;
    }

    let mut info = ConsoleScreenBufferInfo {
        dw_size: Coord { x: 0, y: 0 },
        dw_cursor_position: Coord { x: 0, y: 0 },
        w_attributes: 0,
        sr_window: SmallRect {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        },
        dw_maximum_window_size: Coord { x: 0, y: 0 },
    };
    let ok = unsafe { GetConsoleScreenBufferInfo(handle, &mut info) };
    if ok == 0 {
        return None;
    }

    Some(console_attributes_background_is_black(info.w_attributes))
}

#[cfg(windows)]
pub(super) fn console_attributes_background_is_black(attributes: u16) -> bool {
    const BACKGROUND_MASK: u16 = 0x00F0;
    (attributes & BACKGROUND_MASK) == 0
}
