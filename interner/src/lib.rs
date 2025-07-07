use std::sync::LazyLock;
use lasso::ThreadedRodeo;


pub static INTERN: LazyLock<ThreadedRodeo> = LazyLock::new(ThreadedRodeo::new);