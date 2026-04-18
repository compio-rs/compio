pub fn stub_error() -> std::io::Error {
    std::io::Error::other("Stub driver does not support any operations")
}

pub fn stub_unimpl() -> ! {
    unimplemented!("Stub driver does not support any operations")
}
