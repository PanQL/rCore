use alloc::{boxed::Box, collections::BTreeMap, string::String, sync::Arc, sync::Weak, vec::Vec};
use core::fmt;

use core::str;
use log::*;
use rcore_memory::{Page, PAGE_SIZE};
use rcore_thread::Tid;
use spin::RwLock;
use xmas_elf::{
    header,
    program::{Flags, SegmentData, Type},
    ElfFile,
};

use crate::arch::interrupt::{Context, TrapFrame};
use crate::ipc::SemProc;
use crate::memory::{
    ByFrame, Delay, File, GlobalFrameAlloc, KernelStack, MemoryAttr, MemorySet, Read,
};
use crate::sync::{Condvar, SpinNoIrqLock as Mutex};

use super::abi::{self, ProcInitInfo};
use crate::processor;
use core::mem::MaybeUninit;

pub struct Thread {
    context: Context,
    kstack: KernelStack,
    /// Kernel performs futex wake when thread exits.
    /// Ref: [http://man7.org/linux/man-pages/man2/set_tid_address.2.html]
    pub clear_child_tid: usize,
    // This is same as `proc.vm`
    pub vm: Arc<Mutex<MemorySet>>,
    pub proc: Arc<Mutex<Process>>,
}

/// Pid type
/// For strong type separation
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Pid(usize);

impl Pid {
    pub fn get(&self) -> usize {
        self.0
    }

    /// Return whether this pid represents the init process
    pub fn is_init(&self) -> bool {
        self.0 == 0
    }
}

impl fmt::Display for Pid {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

pub struct Process {
    // resources
    pub vm: Arc<Mutex<MemorySet>>,
    // pub files: BTreeMap<usize, FileLike>,
    pub cwd: String,
    // pub exec_path: String,
    futexes: BTreeMap<usize, Arc<Condvar>>,
    pub semaphores: SemProc,

    // relationship
    pub pid: Pid, // i.e. tgid, usually the tid of first thread
    pub parent: Weak<Mutex<Process>>,
    pub children: Vec<Weak<Mutex<Process>>>,
    pub threads: Vec<Tid>, // threads in the same process

    // for waiting child
    pub child_exit: Arc<Condvar>, // notified when the a child process is going to terminate
    pub child_exit_code: BTreeMap<usize, usize>, // child process store its exit code here
}

lazy_static! {
    /// Records the mapping between pid and Process struct.
    pub static ref PROCESSES: RwLock<BTreeMap<usize, Weak<Mutex<Process>>>> =
        RwLock::new(BTreeMap::new());
}

/// Let `rcore_thread` can switch between our `Thread`
impl rcore_thread::Context for Thread {
    unsafe fn switch_to(&mut self, target: &mut dyn rcore_thread::Context) {
        use core::mem::transmute;
        let (target, _): (&mut Thread, *const ()) = transmute(target);
        self.context.switch(&mut target.context);
    }

    fn set_tid(&mut self, tid: Tid) {
        let mut proc = self.proc.lock();
        // add it to threads
        proc.threads.push(tid);
    }
}

impl Thread {
    /// Make a struct for the init thread
    pub unsafe fn new_init() -> Box<Thread> {
        Box::new(Thread {
            context: Context::null(),
            // safety: other fields will never be used
            ..core::mem::MaybeUninit::zeroed().assume_init()
        })
    }

    /// Make a new kernel thread starting from `entry` with `arg`
    pub fn new_kernel(entry: extern "C" fn(usize) -> !, arg: usize) -> Box<Thread> {
        let vm = MemorySet::new();
        let vm_token = vm.token();
        let vm = Arc::new(Mutex::new(vm));
        let kstack = KernelStack::new();
        Box::new(Thread {
            context: unsafe { Context::new_kernel_thread(entry, arg, kstack.top(), vm_token) },
            kstack,
            clear_child_tid: 0,
            vm: vm.clone(),
            // TODO: kernel thread should not have a process
            proc: Process {
                vm,
                // files: BTreeMap::default(),
                cwd: String::from("/"),
                // exec_path: String::new(),
                semaphores: SemProc::default(),
                futexes: BTreeMap::default(),
                pid: Pid(0),
                parent: Weak::new(),
                children: Vec::new(),
                threads: Vec::new(),
                child_exit: Arc::new(Condvar::new()),
                child_exit_code: BTreeMap::new(),
            }
            .add_to_table(),
        })
    }
}

impl Process {
    /// Assign a pid and put itself to global process table.
    fn add_to_table(mut self) -> Arc<Mutex<Self>> {
        let mut process_table = PROCESSES.write();

        // assign pid
        let pid = (0..)
            .find(|i| match process_table.get(i) {
                Some(p) => false,
                _ => true,
            })
            .unwrap();
        self.pid = Pid(pid);

        // put to process table
        let self_ref = Arc::new(Mutex::new(self));
        process_table.insert(pid, Arc::downgrade(&self_ref));

        self_ref
    }

    pub fn get_futex(&mut self, uaddr: usize) -> Arc<Condvar> {
        if !self.futexes.contains_key(&uaddr) {
            self.futexes.insert(uaddr, Arc::new(Condvar::new()));
        }
        self.futexes.get(&uaddr).unwrap().clone()
    }

    /// Exit the process.
    /// Kill all threads and notify parent with the exit code.
    pub fn exit(&mut self, exit_code: usize) {
        // quit all threads
        for tid in self.threads.iter() {
            processor().manager().exit(*tid, 1);
        }
        // notify parent and fill exit code
        if let Some(parent) = self.parent.upgrade() {
            let mut parent = parent.lock();
            parent.child_exit_code.insert(self.pid.get(), exit_code);
            parent.child_exit.notify_one();
        }
    }
}
