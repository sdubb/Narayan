use std::collections::VecDeque;

#[derive(Clone)]
pub struct SwarmTask {
    pub id: String,
    pub description: String,
}

pub struct SwarmScheduler {
    queue: VecDeque<SwarmTask>,
}

impl SwarmScheduler {
    pub fn new() -> Self {
        Self { queue: VecDeque::new() }
    }

    pub fn push(&mut self, task: SwarmTask) {
        self.queue.push_back(task);
    }

    pub fn next(&mut self) -> Option<SwarmTask> {
        self.queue.pop_front()
    }

    pub fn depth(&self) -> usize {
        self.queue.len()
    }
}

impl Default for SwarmScheduler {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task(id: &str, description: &str) -> SwarmTask {
        SwarmTask { id: id.into(), description: description.into() }
    }

    #[test]
    fn test_swarm_scheduler_is_fifo() {
        let mut scheduler = SwarmScheduler::new();
        scheduler.push(task("a", "first"));
        scheduler.push(task("b", "second"));

        let first = scheduler.next().expect("first task should exist");
        let second = scheduler.next().expect("second task should exist");

        assert_eq!(first.id, "a");
        assert_eq!(first.description, "first");
        assert_eq!(second.id, "b");
        assert_eq!(scheduler.depth(), 0);
    }

    #[test]
    fn test_swarm_scheduler_depth_tracks_queue_size() {
        let mut scheduler = SwarmScheduler::default();
        assert_eq!(scheduler.depth(), 0);
        scheduler.push(task("a", "first"));
        scheduler.push(task("b", "second"));
        assert_eq!(scheduler.depth(), 2);
        let _ = scheduler.next();
        assert_eq!(scheduler.depth(), 1);
    }
}
