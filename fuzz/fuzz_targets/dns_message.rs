#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if data.len() <= 16384 {
        let _ = hickory_proto::op::Message::from_vec(data);
    }
});
