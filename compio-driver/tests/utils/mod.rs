use compio_buf::arrayvec::ArrayVec;
use compio_driver::{OpCode, Proactor, PushEntry};

pub fn push_and_wait<O: OpCode + 'static>(driver: &mut Proactor, op: O) -> (usize, O) {
    match driver.push(op) {
        PushEntry::Ready(res) => res.unwrap(),
        PushEntry::Pending(user_data) => {
            let mut entries = ArrayVec::<usize, 1>::new();
            while entries.is_empty() {
                driver.poll(None, &mut entries).unwrap();
            }
            assert_eq!(entries[0], *user_data);
            driver.pop(user_data).unwrap()
        }
    }
}
