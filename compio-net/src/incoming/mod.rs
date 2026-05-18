cfg_select! {
    windows => {
        #[path = "windows.rs"]
        mod sys;
    }
    unix => {
        #[path = "unix.rs"]
        mod sys;
    }
    _ => {}
}

pub use sys::*;
