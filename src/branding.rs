#![forbid(unsafe_code)]

// Shared by the editor and build tools, which have different Cargo package names.
macro_rules! display_name {
    () => {
        "Nero"
    };
}

macro_rules! executable_name {
    () => {
        "nero"
    };
}

pub(crate) use display_name;
pub(crate) use executable_name;

pub const DISPLAY_NAME: &str = self::display_name!();
pub const EXECUTABLE_NAME: &str = self::executable_name!();
