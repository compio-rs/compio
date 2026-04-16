#[derive(Debug)]
pub(in crate::sys) struct Extra {}
pub(in crate::sys) use Extra as StubExtra;

impl Extra {
    pub fn new() -> Self {
        Self {}
    }
}
