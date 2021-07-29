#[derive(Debug, PartialEq, Eq)]
#[repr(i64)]
pub enum State {
    None,
    Waiter,
    Reader,
    Writer,
}
