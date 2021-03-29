use crate::{CurrentTask, TaskBuilder, TaskRegistry};

#[doc(hidden)]
pub struct BuildFn(pub for<'a> fn(&'a mut TaskBuilder<'static>) -> &'a mut TaskBuilder<'static>);
#[doc(hidden)]
pub struct RunFn(pub fn(&TaskRegistry, CurrentTask));
