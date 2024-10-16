#[derive(Debug, Clone, Default)]
pub struct Value<T> {
    pub id: String,
    pub value: T,
}

impl<T> Value<T> {
    pub fn new(id: String, value: T) -> Self {
        Self { id, value }
    }
}
