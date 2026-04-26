use std::process::{Child, Command};

pub struct TestChild {
    child: Child,
}

impl TestChild {
    pub fn spawn(mut command: Command) -> Self {
        let child = command.spawn().expect("spawn test child");
        Self { child }
    }
}

impl std::ops::Deref for TestChild {
    type Target = Child;

    fn deref(&self) -> &Self::Target {
        &self.child
    }
}

impl std::ops::DerefMut for TestChild {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.child
    }
}

impl Drop for TestChild {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
