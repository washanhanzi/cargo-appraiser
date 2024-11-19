#[derive(Debug, Clone, Default)]
pub struct Value<T> {
    id: String,
    value: T,
}

impl<T> Value<T> {
    pub fn new(id: String, value: T) -> Self {
        Self { id, value }
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn value(&self) -> &T {
        &self.value
    }
}
