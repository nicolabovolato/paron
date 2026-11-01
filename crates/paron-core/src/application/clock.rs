pub trait Clock: Send + Sync {
    fn now(&self) -> jiff::Timestamp;
}
