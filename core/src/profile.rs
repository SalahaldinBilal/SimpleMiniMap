use std::cell::RefCell;
use std::time::{Duration, Instant};

use crate::host;

const SHOW_FROM: Duration = Duration::from_millis(1);

struct Section {
    name: &'static str,
    parent: Option<usize>,
    calls: u32,
    total: Duration,
    worst: Duration,
}

#[derive(Default)]
struct Profile {
    sections: Vec<Section>,
    open: Vec<usize>,
}

impl Profile {
    fn enter(&mut self, name: &'static str) -> usize {
        let parent = self.open.last().copied();
        let index = match self
            .sections
            .iter()
            .position(|section| section.parent == parent && section.name == name)
        {
            Some(index) => index,
            None => {
                self.sections.push(Section {
                    name,
                    parent,
                    calls: 0,
                    total: Duration::ZERO,
                    worst: Duration::ZERO,
                });
                self.sections.len() - 1
            }
        };
        self.open.push(index);
        index
    }

    fn leave(&mut self, index: usize, took: Duration) {
        if let Some(at) = self.open.iter().rposition(|&open| open == index) {
            self.open.truncate(at);
        }
        let section = &mut self.sections[index];
        section.calls += 1;
        section.total += took;
        section.worst = section.worst.max(took);
    }

    fn write(&self, parent: Option<usize>, depth: usize, lines: &mut Vec<String>) {
        let mut children: Vec<usize> = (0..self.sections.len())
            .filter(|&index| self.sections[index].parent == parent)
            .collect();
        children.sort_by_key(|&index| std::cmp::Reverse(self.sections[index].total));
        for index in children {
            let section = &self.sections[index];
            if section.total < SHOW_FROM && section.worst < SHOW_FROM {
                continue;
            }
            lines.push(format!(
                "{:indent$}{} {:.1} ms in {} calls, worst {:.1} ms",
                "",
                section.name,
                section.total.as_secs_f64() * 1e3,
                section.calls,
                section.worst.as_secs_f64() * 1e3,
                indent = 2 + depth * 2
            ));
            self.write(Some(index), depth + 1, lines);
        }
    }
}

thread_local! {
    static PROFILE: RefCell<Profile> = RefCell::new(Profile::default());
}

/// Times its section until dropped. Sections opened inside it are reported under it.
pub struct Scope {
    index: usize,
    started: Instant,
}

impl Drop for Scope {
    fn drop(&mut self) {
        let took = self.started.elapsed();
        PROFILE.with_borrow_mut(|profile| profile.leave(self.index, took));
    }
}

pub fn scope(name: &'static str) -> Scope {
    let index = PROFILE.with_borrow_mut(|profile| profile.enter(name));
    Scope {
        index,
        started: Instant::now(),
    }
}

pub fn time<T>(name: &'static str, work: impl FnOnce() -> T) -> T {
    let _scope = scope(name);
    work()
}

/// Logs where the time went since the last report, as a tree, then starts counting again.
pub fn report(window: Duration) {
    PROFILE.with_borrow_mut(|profile| {
        let mut lines = Vec::new();
        profile.write(None, 0, &mut lines);
        host::info(&format!(
            "time spent in the last {:.0} s, sections under 1 ms left out:",
            window.as_secs_f64()
        ));
        for line in &lines {
            host::info(line);
        }
        for section in &mut profile.sections {
            section.calls = 0;
            section.total = Duration::ZERO;
            section.worst = Duration::ZERO;
        }
    });
}
