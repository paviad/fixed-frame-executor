use {
  futures::{
    future::{BoxFuture, FutureExt},
    task::{waker_ref, ArcWake},
  },
  std::{
    future::Future,
    sync::{Arc, Mutex},
    task::{Context, Poll},
    time::{Duration, Instant},
  },
};

static mut ITER: u32 = 0;
static mut WORK_DONE: u32 = 0;

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

fn incr_work() {
  unsafe {
    WORK_DONE += 1;
  }
}

fn reset_work() {
  unsafe {
    WORK_DONE = 0;
  }
}

fn get_work() -> u32 {
  unsafe { WORK_DONE }
}

impl Future for WaitFrames {
  type Output = ();
  fn poll(
    self: std::pin::Pin<&mut Self>,
    _cx: &mut std::task::Context<'_>,
  ) -> std::task::Poll<<Self as std::future::Future>::Output> {
    let iter = get_iter();
    if iter >= self.frame {
      incr_work();
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
  frame_start: Option<Instant>,
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

impl ArcWake for Task {
  fn wake_by_ref(_arc_self: &Arc<Self>) {}
}

impl Executor {
  fn new() -> Self {
    Executor {
      tasks: vec![],
      frame_start: None,
    }
  }

  fn spawn(&mut self, future: impl Future<Output = ()> + 'static + Send) {
    let future = future.boxed();
    let task = Arc::new(Task {
      future: Mutex::new(Some(future)),
    });
    self.tasks.push(Arc::clone(&task));
  }

  fn begin_frame(&mut self) {
    self.frame_start = Some(Instant::now());
  }

  fn end_frame(&mut self) {
    let dur = Duration::from_secs_f64(1. / 60.);

    incr_iter();
    let busy_time = Instant::now().duration_since(self.frame_start.unwrap());
    let delta = if dur > busy_time {
      dur - busy_time
    } else {
      Duration::ZERO
    };
    let futures_run = get_work();

    if futures_run > 0 {
      eprintln!(
        "busy frame took {:?} ({} ready futures)",
        busy_time, futures_run
      );
      reset_work();
    }
    spin_sleep::sleep(delta);
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
      println!("entry1 howdy! waiting 1 second tick {}", get_iter());
      // Wait for our timer future to complete after two seconds.
      WaitFrames::new(60).await;
      println!("entry1 howdy! waiting 1 second tock {}", get_iter());
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
  let mut executor = Executor::new();

  // Add all tasks to the executor
  executor.spawn(entry1());
  executor.spawn(entry2());

  // Run the executor every frame
  loop {
    executor.begin_frame();
    executor.run();
    executor.end_frame();
  }
}
