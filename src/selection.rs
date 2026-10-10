pub enum Selector {
    AwaitingClick,
    Clicked{start: (f64, f64)},
}
