use crate::println;

pub extern "C" fn dispatch(syscall_number: u64) -> i64 {
    if syscall_number == 1 { println!("Hello!") }

    0
}