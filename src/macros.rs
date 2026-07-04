#[macro_export]
macro_rules! mock_resource {
    ($fname:expr) => {
        concat!(env!("CARGO_MANIFEST_DIR"), "/resources/mock/", $fname)
    };
}
