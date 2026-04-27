extern crate exitcode;
use std::error::Error;
use std::fmt;

pub(crate) mod frame_exitcode {
    pub type FrameExitCode = i32;

    /// Framepiler parse error exit
    pub const PARSE_ERR: FrameExitCode = 1;
    pub const CONFIG_ERR: FrameExitCode = 2;

    pub fn as_string(code: FrameExitCode) -> String {
        match code {
            PARSE_ERR => "Frame parse error".to_string(),
            CONFIG_ERR => "Configuration error".to_string(),
            _ => format!("Unknown error code {}", code),
        }
    }
}

pub struct RunError {
    pub code: frame_exitcode::FrameExitCode,
    pub error: String,
}

impl RunError {
    pub fn new(code: frame_exitcode::FrameExitCode, msg: &str) -> RunError {
        RunError {
            code,
            error: String::from(msg),
        }
    }
}

impl fmt::Display for RunError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "{}: {}",
            frame_exitcode::as_string(self.code),
            self.error
        )
    }
}

impl fmt::Debug for RunError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "{}: {}",
            frame_exitcode::as_string(self.code),
            self.error
        )
    }
}

impl Error for RunError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        None
    }
}
