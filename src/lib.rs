extern crate js_sys;
//https://medium.com/@ffimnsr/creating-a-cloudflare-worker-using-rust-for-fetching-resources-1973871eb6c4
//https://github.com/ffimnsr-ext/cloudflare-rust-fetch-demo-1973871eb6c4
use cfg_if::cfg_if;
//use serde::{Deserialize, Serialize};
use wasm_bindgen::prelude::*;
use log::Level;
//use wasm_bindgen::{JsCast};
//use wasm_bindgen_futures::JsFuture;
//use web_sys::{Request , RequestInit, Response }; //info,
use js_sys::Promise;
mod utils;

cfg_if! {
    // When the `wee_alloc` feature is enabled, use `wee_alloc` as the global
    // allocator.
    if #[cfg(feature = "wee_alloc")] {
        extern crate wee_alloc;
        #[global_allocator]
        static ALLOC: wee_alloc::WeeAlloc = wee_alloc::WeeAlloc::INIT;
    }
}

cfg_if! {
    if #[cfg(feature = "console_log")] {
        fn init_log() {
            console_log::init_with_level(Level::Trace).expect("error initializing log");
        }
    } else {
        fn init_log() {}
    }
}

use std::sync::atomic::{AtomicBool, Ordering};
use worker::{
    event,
    wasm_bindgen::JsValue,
    wasm_bindgen_futures, //::future_to_promise//
    //Env,
    //Request,
    //RequestInit,
    //Response,
    Result,
    //Router, //, Url,
};
//use wasm_bindgen_futures::{JsFuture, future_to_promise};
//use wasm_bindgen_macro_support;

//mod index;
//mod utils;
static GLOBAL_STATE: AtomicBool = AtomicBool::new(false);
#[event(start)]
pub fn startt() {
    utils::set_panic_hook();
    GLOBAL_STATE.store(true, Ordering::SeqCst);
}

/*struct SomeSharedData {
    //data: u8, //regex::Regex,
}*/
use serde::Serialize;
use std::{
    collections::HashMap,
    future::Future,
    mem,
    pin::Pin,
    sync::mpsc::{channel, Sender},
    sync::{Arc, Mutex},
    task::{Context, Poll, Waker},
    thread::{self, JoinHandle},
    time::Duration,
};
#[derive(Serialize)]
struct Product {
    ivity: String,
}
#[wasm_bindgen]
pub async fn main(/*req: Request, env: Env, _ctx: worker::Context*/) -> Result<Promise> {
    std::panic::set_hook(Box::new(console_error_panic_hook::hook));
    init_log();

    //Ok(json)

    //use the request parameter of the router instead else return closure/(pointer)
    /*fn origin_url(req_headers: &worker::Headers) -> std::string::String {
        return match req_headers.get("Origin").unwrap() {
            Some(value) => value,
            None => "".to_owned() + "", //Response::empty(),
        };
    }
    let info = SomeSharedData {
        //data: 0, //regex::Regex::new(r"^\d{4}-\d{2}-\d{2}$").unwrap(),
    };*/

    //(1) Reactor
    // The different states a task can have in this Reactor
    enum TaskState {
        Ready,
        NotReady(Waker),
        Finished,
    } //https://cfsamson.github.io/books-futures-explained/6_future_example.html

    // This is a "fake" reactor. It does no real I/O, but that also makes our
    // code possible to run in the book and in the playground
    struct Reactor {
        // we need some way of registering a Task with the reactor. Normally this
        // would be an "interest" in an I/O event
        dispatcher: Sender<Event>,
        handle: Option<JoinHandle<()>>,

        // This is a list of tasks
        tasks: HashMap<usize, TaskState>,
    }

    // This represents the Events we can send to our reactor thread. In this
    // example it's only a Timeout or a Close event.
    #[derive(Debug)]
    enum Event {
        Close,
        Timeout(u64, usize),
    }

    impl Reactor {
        // atomic reference counted, mutex protected, heap allocated `Reactor`.
        // 1. only thread-safe reactors will be created.
        // 2. heap allocating obtains a stable address reference
        // not dependent on function stack frame caller of `new`
        fn new() -> Arc<Mutex<Box<Self>>> {
            let (tx, rx) = channel::<Event>();
            let reactor = Arc::new(Mutex::new(Box::new(Reactor {
                dispatcher: tx,
                handle: None,
                tasks: HashMap::new(),
            })));

            // `weak` reference so our `Reactor` will not get `dropped`
            // when our main thread is finished for the Reactor as internal references.

            // collect `JoinHandles` from threads spawned & joined
            // `Reactor` will be alive longer than any reference held by the threads we spawn here.
            let reactor_clone = Arc::downgrade(&reactor);

            // 'Reactor-thread'. now merely spawn new thread timers.
            let handle = thread::spawn(move || {
                let mut handles = vec![];

                // This simulates some I/O resource
                for event in rx {
                    println!("REACTOR: {:?}", event);
                    let reactor = reactor_clone.clone();
                    match event {
                        Event::Close => break,
                        Event::Timeout(duration, id) => {
                            // serve spawn new thread timer & call `Waker` `wake` when done.
                            let event_handle = thread::spawn(move || {
                                thread::sleep(Duration::from_secs(duration));
                                let reactor = reactor.upgrade().unwrap();
                                reactor.lock().map(|mut r| r.wake(id)).unwrap();
                            });
                            handles.push(event_handle);
                        }
                    }
                }

                // This is important for us since we need to know that these
                // threads don't live longer than our Reactor-thread. Our
                // Reactor-thread will be joined when `Reactor` gets dropped.
                handles
                    .into_iter()
                    .for_each(|handle| handle.join().unwrap());
            });
            reactor.lock().map(|mut r| r.handle = Some(handle)).unwrap();
            reactor
        }

        // The wake function will call wake on the waker for the task with the
        // corresponding id.
        fn wake(&mut self, id: usize) {
            self.tasks
                .get_mut(&id)
                .map(|state| {
                    // No matter what state the task was in we can safely set it
                    // to ready at this point. This lets us get ownership over the
                    // the data that was there before we replaced it.
                    match mem::replace(state, TaskState::Ready) {
                        TaskState::NotReady(waker) => waker.wake(),
                        TaskState::Finished => {
                            panic!("Called 'wake' twice on task: {}", id)
                        }
                        _ => unreachable!(),
                    }
                })
                .unwrap();
        }

        // Register a new task with the reactor. In this particular example
        // we panic if a task with the same id get's registered twice
        fn register(&mut self, duration: u64, waker: Waker, id: usize) {
            if self.tasks.insert(id, TaskState::NotReady(waker)).is_some() {
                panic!("Tried to insert a task with id: '{}', twice!", id);
            }
            self.dispatcher.send(Event::Timeout(duration, id)).unwrap();
        }

        // We simply checks if a task with this id is in the state `TaskState::Ready`
        fn is_ready(&self, id: usize) -> bool {
            self.tasks
                .get(&id)
                .map(|state| match state {
                    TaskState::Ready => true,
                    _ => false,
                })
                .unwrap_or(false)
        }
    }

    impl Drop for Reactor {
        fn drop(&mut self) {
            // We send a close event to the reactor so it closes down our reactor-thread.
            // If we don't do that we'll end up waiting forever for new events.
            self.dispatcher.send(Event::Close).unwrap();
            self.handle.take().map(|h| h.join().unwrap()).unwrap();
        }
    }
    //(2) Task
    #[derive(Clone)]
    pub struct Task {
        app: u64,
        reactor: Arc<Mutex<Box<Reactor>>>,
        id: usize,
    }
    impl Task {
        fn new(pathstr: &str, reactor: Arc<Mutex<Box<Reactor>>>, id: usize) -> Self {
            //Task {
            Task {
                app: match pathstr {
                    //let s: String = match pathstr {
                    "/" => {
                        fn pathify(path: &str) -> std::path::PathBuf {
                            let mut input_file = std::path::PathBuf::new();
                            let _arr: () = path.split("/").map(|x| input_file.push(x)).collect();
                            return input_file;
                        }
                        let lock: std::path::PathBuf = pathify("./exec.c");
                        let appel = cc::Build::new().file(lock).expand();
                        //String::from_utf8(appel).unwrap()
                        u64::from_be_bytes(appel.try_into().expect(""))
                        //.iter().collect()
                    }
                    &_ => u64::from_be_bytes("".as_bytes().try_into().expect("")),
                    //};
                    //u64::from_str_radix(s.expect("")) //,16
                },
                reactor,
                id,
            }
        }
    }
    // (3) Future implementation
    impl Future for Task {
        type Output = usize;

        fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
            //let mut r = self.app;
            let mut r = self.reactor.lock().unwrap();
            if r.is_ready(self.id) {
                *r.tasks.get_mut(&self.id).unwrap() = TaskState::Finished;
                Poll::Ready(self.id)
            } else if r.tasks.contains_key(&self.id) {
                r.tasks
                    .insert(self.id, TaskState::NotReady(cx.waker().clone()));
                Poll::Pending
            } else {
                r.register(self.app, cx.waker().clone(), self.id);
                Poll::Pending
            }
        }
    }
    let path = "/";//req.path(); //longer lived with let
    let pathstr: &str = path;//.as_str();
    let reactor = Reactor::new();
    let id = 1;
    let future1 = Task::new(pathstr, reactor.clone(), id);

    let promise =
        wasm_bindgen_futures::future_to_promise(async { Ok(JsValue::from(future1.await)) });
    //Ok(promise.into_serde().unwrap())
    //Ok(wasm_bindgen_futures::JsFuture::from(promise))
    Ok(promise)
}
//https://github.com/rustwasm/wasm-bindgen/issues/1127
//as soon an the store is open, haha you know what. wow, this is high quality stuff dood
//she taught kids, take a bath lady. what can I tell ya Trumps high school he misbehaved and went to military
//"can't be beta, 'I got clinked in the head by a hammer'"
//we are just closing stuff here
