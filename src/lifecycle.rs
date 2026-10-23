#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Work {
    Mining,
    Benchmark,
}
#[derive(Default)]
// Start a replacement only after the previous worker reports Finished.
pub struct Lifecycle {
    pub work: Option<Work>,
    pub want_mining: bool,
    pub stopping: bool,
}
impl Lifecycle {
    pub fn start(&mut self) {
        self.want_mining = true;
    }
    pub fn stop(&mut self) {
        self.want_mining = false;
        self.stopping = self.work.is_some();
    }
    pub fn reconfigure(&mut self, changed: bool) -> bool {
        let restart = changed && self.work == Some(Work::Mining) && self.want_mining;
        if restart {
            self.stopping = true;
        }
        restart
    }
    pub fn launched(&mut self, work: Work) {
        assert!(self.work.is_none());
        self.work = Some(work);
        self.stopping = false;
    }
    pub fn finished(&mut self, failed: bool) -> bool {
        if failed && !self.stopping {
            self.want_mining = false;
        }
        self.work = None;
        self.stopping = false;
        self.want_mining
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn applying_settings_retires_before_relaunch_and_stop_cancels() {
        let mut l = Lifecycle::default();
        l.start();
        l.launched(Work::Mining);
        assert!(l.reconfigure(true));
        assert_eq!(l.work, Some(Work::Mining));
        assert!(l.reconfigure(true));
        assert!(l.finished(false));
        l.launched(Work::Mining);
        assert!(l.reconfigure(true));
        l.stop();
        assert!(!l.finished(false));
    }
    #[test]
    fn appearance_changes_and_benchmarks_do_not_start_mining() {
        let mut l = Lifecycle::default();
        l.launched(Work::Benchmark);
        assert!(!l.reconfigure(true));
        assert!(!l.finished(false));
        l.start();
        l.launched(Work::Mining);
        assert!(!l.reconfigure(false));
        assert!(!l.stopping);
        assert!(!l.finished(true));
    }
}

#[cfg(test)]
mod restart_tests {
    use super::*;
    #[test]
    fn latest_start_survives_retiring_benchmark_and_stop_survives_completion() {
        let mut l = Lifecycle::default();
        l.launched(Work::Benchmark);
        l.start();
        l.stopping = true;
        assert!(l.finished(false));
        l.launched(Work::Mining);
        l.stop();
        l.reconfigure(true);
        assert!(!l.finished(false));
        assert!(!l.want_mining);
    }
}
