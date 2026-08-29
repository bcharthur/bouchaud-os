// Public result types.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WaitWordWake {
    Signaled,
    ValueChanged,
    Deadline,
    Fault,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct WaitWordStats {
    pub waits: u64,
    pub value_changed: u64,
    pub signaled: u64,
    pub deadlines: u64,
    pub faults: u64,
    pub wakes: u64,
    pub wake_misses: u64,
    pub entries_created: u64,
    pub entries_pruned: u64,
    pub bucket_peak: u64,
}
