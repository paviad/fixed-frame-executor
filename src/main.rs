use {
  futures::{
    future::{BoxFuture, FutureExt},
    task::{waker_ref, ArcWake},
  },
  std::{
    future::Future,
    sync::{Arc, Mutex},
    task::{Context, Poll},
    time::Duration,
  },
};

static mut ITER: u32 = 0;

pub struct WaitFrames {
  frame: u32,
}

fn get_iter() -> u32 {
  unsafe { ITER }
}

fn incr_iter() {
  unsafe {
    ITER += 1;
  }
}

impl Future for WaitFrames {
  type Output = ();
  fn poll(
    self: std::pin::Pin<&mut Self>,
    _cx: &mut std::task::Context<'_>,
  ) -> std::task::Poll<<Self as std::future::Future>::Output> {
    let iter = get_iter();
    if iter >= self.frame {
      Poll::Ready(())
    } else {
      Poll::Pending
    }
  }
}

impl WaitFrames {
  fn new(frames: u32) -> Self {
    let iter = get_iter();
    WaitFrames {
      frame: iter + frames,
    }
  }
}

/// Task executor that receives tasks off of a channel and runs them.
struct Executor {
  tasks: Vec<Arc<Task>>,
}

/// A future that can reschedule itself to be polled by an `Executor`.
struct Task {
  /// In-progress future that should be pushed to completion.
  ///
  /// The `Mutex` is not necessary for correctness, since we only have
  /// one thread executing tasks at once. However, Rust isn't smart
  /// enough to know that `future` is only mutated from one thread,
  /// so we need to use the `Mutex` to prove thread-safety. A production
  /// executor would not need this, and could use `UnsafeCell` instead.
  future: Mutex<Option<BoxFuture<'static, ()>>>,
}

fn new_executor_and_spawner() -> Executor {
  // Maximum number of tasks to allow queueing in the channel at once.
  // This is just to make `sync_channel` happy, and wouldn't be present in
  // a real executor.
  Executor { tasks: vec![] }
}

impl ArcWake for Task {
  fn wake_by_ref(_arc_self: &Arc<Self>) {}
}

impl Executor {
  fn spawn(&mut self, future: impl Future<Output = ()> + 'static + Send) {
    let future = future.boxed();
    let task = Arc::new(Task {
      future: Mutex::new(Some(future)),
    });
    self.tasks.push(Arc::clone(&task));
  }
  fn run(&self) {
    for task in self.tasks.iter() {
      // Take the future, and if it has not yet completed (is still Some),
      // poll it in an attempt to complete it.
      let mut future_slot = task.future.lock().unwrap();
      if let Some(mut future) = future_slot.take() {
        // Create a `LocalWaker` from the task itself
        let waker = waker_ref(&task);
        let context = &mut Context::from_waker(&waker);
        // `BoxFuture<T>` is a type alias for
        // `Pin<Box<dyn Future<Output = T> + Send + 'static>>`.
        // We can get a `Pin<&mut dyn Future + Send + 'static>`
        // from it by calling the `Pin::as_mut` method.
        if future.as_mut().poll(context).is_pending() {
          // We're not done processing the future, so put it
          // back in its task to be run again in the future.
          *future_slot = Some(future);
        }
      }
    }
  }
}

async fn entry1() {
  let a1 = async {
    loop {
      println!("entry1 howdy! waiting 1 second {}", get_iter());
      // Wait for our timer future to complete after two seconds.
      WaitFrames::new(60).await;
      println!("entry1 howdy! waiting 1 second (again) {}", get_iter());
      WaitFrames::new(60).await;
    }
  };
  let a2 = async {
    loop {
      println!("entry1 howdy! waiting 2 seconds {}", get_iter());
      // Wait for our timer future to complete after two seconds.
      WaitFrames::new(120).await;
    }
  };
  futures::join!(a1, a2);
}

async fn entry2() {
  loop {
    println!("entry2 howdy! waiting 2 seconds {}", get_iter());
    // Wait for our timer future to complete after two seconds.
    WaitFrames::new(120).await;
  }
}

pub fn main() {
  let mut executor = new_executor_and_spawner();

  // Add all tasks to the executor
  executor.spawn(entry1());
  executor.spawn(entry2());

  let dur = Duration::from_secs_f64(1. / 60.);

  // Run the executor every frame
  loop {
    executor.run();
    incr_iter();
    spin_sleep::sleep(dur);
  }
}
