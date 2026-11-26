//! L4: optional world streaming. Enabled in the engine with the `streaming` feature.
//!
//! A world is split into areas, each kept in its own scene file and marked in the playing scene by
//! a Streamed Area component. [`Streamer`] decides from the cameras' places which areas should be
//! there, reads their files on worker threads so play does not wait, and hands back the scenes
//! that arrived for the engine to add. It keeps a margin so an area at the edge does not come and
//! go every step.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use std::collections::{BTreeMap, VecDeque};
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, TryRecvError};
use std::time::{Duration, Instant};

use bayleaf_core::StableId;
use bayleaf_core::math::DVec3;
use bayleaf_scene::Scene;

/// Most scene files read at once; the others wait their turn.
pub const MAX_READING: usize = 4;

/// A Streamed Area in the playing scene, as this step sees it.
#[derive(Clone, Debug, PartialEq)]
pub struct Area {
    /// The object with the component.
    pub id: StableId,
    /// Its world position.
    pub at: DVec3,
    /// Its scene file, relative to the project folder; empty for none.
    pub scene: String,
    pub load_distance: f64,
    pub keep_margin: f64,
}

/// Where an area that is not away is.
#[derive(Clone, Debug, PartialEq)]
pub enum State {
    /// Waiting for its turn to be read.
    Waiting,
    /// Its file is being read.
    Reading,
    /// Added: the top objects of its scene, under the area's object.
    Here(Vec<StableId>),
    /// Its scene could not be read or added; tried again once the cameras leave and come back, or
    /// its scene changes.
    Failed(String),
}

impl State {
    /// A one-word name for agents: `waiting`, `reading`, `here` or `failed`.
    pub fn word(&self) -> &'static str {
        match self {
            State::Waiting => "waiting",
            State::Reading => "reading",
            State::Here(_) => "here",
            State::Failed(_) => "failed",
        }
    }
}

/// What the engine does after [`Streamer::plan`].
#[derive(Debug, Default, PartialEq)]
pub struct Plan {
    /// Areas whose scene file should now be read: the area and its scene, relative to the project.
    pub read: Vec<(StableId, String)>,
    /// Areas to take away: the area and its scene's top objects to delete (some may be gone already).
    pub remove: Vec<(StableId, Vec<StableId>)>,
}

/// A scene read for an area, or why it could not be.
pub type Arrival = (StableId, Result<Scene, String>);

struct Entry {
    scene: String,
    state: State,
    reading: Option<Receiver<Result<Scene, String>>>,
}

/// Keeps track of every area that is not away.
#[derive(Default)]
pub struct Streamer {
    entries: BTreeMap<StableId, Entry>,
    queue: VecDeque<(StableId, PathBuf)>,
}

impl Streamer {
    pub fn new() -> Self {
        Self::default()
    }

    /// Compares the areas with the cameras' places. An area comes when a camera is within its Load
    /// Distance and goes when every camera is beyond Load Distance plus Keep Margin; areas whose
    /// object or component went, or whose scene changed, go too. With no camera, none comes or goes
    /// by distance.
    pub fn plan(&mut self, areas: &[Area], cameras: &[DVec3]) -> Plan {
        let mut plan = Plan::default();
        let current: BTreeMap<StableId, &Area> = areas.iter().map(|a| (a.id, a)).collect();
        let gone: Vec<StableId> =
            self.entries.iter().filter(|(id, entry)| current.get(id).is_none_or(|area| area.scene != entry.scene)).map(|(id, _)| *id).collect();
        for id in gone {
            plan.remove.push((id, self.forget(id)));
        }
        if cameras.is_empty() {
            return plan;
        }
        for area in areas {
            if area.scene.is_empty() {
                continue;
            }
            let nearest = cameras.iter().map(|c| c.distance(area.at)).fold(f64::INFINITY, f64::min);
            let known = self.entries.contains_key(&area.id);
            if !known && nearest <= area.load_distance {
                self.entries.insert(area.id, Entry { scene: area.scene.clone(), state: State::Waiting, reading: None });
                plan.read.push((area.id, area.scene.clone()));
            } else if known && nearest > area.load_distance + area.keep_margin {
                plan.remove.push((area.id, self.forget(area.id)));
            }
        }
        plan
    }

    /// Starts reading an area's scene file (already checked to be allowed), now or when a reader is free.
    pub fn read(&mut self, id: StableId, path: PathBuf) {
        if self.entries.contains_key(&id) {
            self.queue.push_back((id, path));
            self.start_waiting();
        }
    }

    /// Marks an area as failed: its scene could not be read or added.
    pub fn fail(&mut self, id: StableId, why: String) {
        if let Some(entry) = self.entries.get_mut(&id) {
            entry.state = State::Failed(why);
            entry.reading = None;
        }
        self.queue.retain(|(queued, _)| *queued != id);
        self.start_waiting();
    }

    /// Records the objects an arrived area's scene became.
    pub fn placed(&mut self, id: StableId, roots: Vec<StableId>) {
        if let Some(entry) = self.entries.get_mut(&id) {
            entry.state = State::Here(roots);
        }
    }

    /// The scenes read since the last call, without waiting. Each is still to be added, or failed.
    pub fn arrived(&mut self) -> Vec<Arrival> {
        let mut out = Vec::new();
        for (id, entry) in &mut self.entries {
            let Some(reading) = &entry.reading else { continue };
            let result = match reading.try_recv() {
                Ok(result) => result,
                Err(TryRecvError::Empty) => continue,
                Err(TryRecvError::Disconnected) => Err("its reader stopped".to_owned()),
            };
            entry.reading = None;
            out.push((*id, result));
        }
        self.settle(&out);
        out
    }

    /// Waits until no area is waiting or being read, or `timeout` passes, and returns what arrived.
    pub fn wait(&mut self, timeout: Duration) -> Vec<Arrival> {
        let deadline = Instant::now() + timeout;
        let mut out = self.arrived();
        while let Some((id, reading)) = self.entries.iter().find_map(|(id, e)| e.reading.as_ref().map(|r| (*id, r))) {
            let result = match reading.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
                Ok(result) => result,
                Err(RecvTimeoutError::Timeout) => break,
                Err(RecvTimeoutError::Disconnected) => Err("its reader stopped".to_owned()),
            };
            if let Some(entry) = self.entries.get_mut(&id) {
                entry.reading = None;
            }
            let one = [(id, result)];
            self.settle(&one);
            out.extend(one);
        }
        out
    }

    /// How many areas are waiting or being read.
    pub fn busy(&self) -> usize {
        self.entries.values().filter(|e| matches!(e.state, State::Waiting | State::Reading)).count()
    }

    /// Every area that is not away, with its scene and state, in id order.
    pub fn states(&self) -> impl Iterator<Item = (StableId, &str, &State)> {
        self.entries.iter().map(|(id, e)| (*id, e.scene.as_str(), &e.state))
    }

    /// Forgets every area, as when the playing scene is replaced.
    pub fn clear(&mut self) {
        self.entries.clear();
        self.queue.clear();
    }

    fn forget(&mut self, id: StableId) -> Vec<StableId> {
        self.queue.retain(|(queued, _)| *queued != id);
        let roots = match self.entries.remove(&id).map(|e| e.state) {
            Some(State::Here(roots)) => roots,
            _ => Vec::new(),
        };
        self.start_waiting();
        roots
    }

    fn settle(&mut self, arrived: &[Arrival]) {
        for (id, result) in arrived {
            if let (Some(entry), Err(why)) = (self.entries.get_mut(id), result) {
                entry.state = State::Failed(why.clone());
            }
        }
        self.start_waiting();
    }

    fn start_waiting(&mut self) {
        while self.entries.values().filter(|e| e.reading.is_some()).count() < MAX_READING {
            let Some((id, path)) = self.queue.pop_front() else { return };
            let Some(entry) = self.entries.get_mut(&id) else { continue };
            let (send, receive) = mpsc::channel();
            let spawned = std::thread::Builder::new().name("bayleaf-streaming".to_owned()).spawn(move || {
                let _ = send.send(Scene::load(&path).map_err(|e| e.to_string()));
            });
            match spawned {
                Ok(_) => {
                    entry.state = State::Reading;
                    entry.reading = Some(receive);
                }
                Err(e) => entry.state = State::Failed(format!("cannot start a reader: {e}")),
            }
        }
    }
}

#[cfg(test)]
mod tests;
