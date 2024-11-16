pub const CARGO: &str = "cargo";

pub fn supported_commands() -> Vec<String> {
    vec![CARGO.to_string()]
}
