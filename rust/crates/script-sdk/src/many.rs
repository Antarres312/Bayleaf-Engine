//! Every object of a script at once, for [`Script::update_many`].

use crate::batch::{CHANGED, CURRENT, RUNNING};
use crate::{Entry, Script, Transform};

/// Every object a module runs a script on, in one step: each one's script and transform, given out
/// together by [`Many::iter_mut`] with no call to the host. In a module that runs many objects the
/// transforms are the step's frame itself; in native mode there is one object, read from the host.
///
/// While items are out, the functions about "this object" ([`crate::property`],
/// [`crate::Object::this`], sending, sounds) are about the object of the item given out last. Its
/// transform is the item's: [`Transform::get`] and [`Transform::set`] there read and write what
/// the object had when the step began, and the item's transform is what the step keeps.
pub struct Many<'a, T: Script> {
    slots: &'a mut [Option<T>],
    /// The objects' entries while items are out; `None` before.
    entries: Option<Vec<Entry>>,
    /// Whether the entries are the frame's (else the one object's, from the host).
    from_frame: bool,
}

impl<'a, T: Script> Many<'a, T> {
    /// The objects of an update of all (used by [`crate::script!`]). Each one whose properties may
    /// have changed reads them first.
    #[doc(hidden)]
    pub fn of_frame(slots: &'a mut [Option<T>]) -> Self {
        #[cfg(target_arch = "wasm32")]
        {
            let mut due = Vec::new();
            crate::batch::take_all_due(&mut due);
            for slot in due {
                if let Some(Some(script)) = slots.get_mut(slot)
                    && crate::batch::select(slot as i32)
                {
                    script.read_properties();
                }
            }
        }
        Self { slots, entries: None, from_frame: true }
    }

    /// One object's script, as native mode runs it.
    pub(crate) fn one(slot: &'a mut Option<T>) -> Self {
        Self { slots: std::slice::from_mut(slot), entries: None, from_frame: false }
    }

    /// Each running object's script and transform. Every object given out counts as moved: the step
    /// keeps the transform it is left with.
    pub fn iter_mut(&mut self) -> impl Iterator<Item = (&mut T, &mut Transform)> + '_ {
        if self.entries.is_none() {
            self.entries = self.take();
        }
        let from_frame = self.from_frame;
        let entries: &mut [Entry] = self.entries.as_deref_mut().unwrap_or(&mut []);
        self.slots.iter_mut().zip(entries).enumerate().filter_map(move |(slot, (script, entry))| {
            let script = script.as_mut()?;
            if entry.flags & RUNNING == 0 {
                return None;
            }
            entry.flags |= CHANGED | CURRENT;
            #[cfg(target_arch = "wasm32")]
            if from_frame {
                crate::batch::item(slot);
            }
            let _ = (slot, from_frame);
            Some((script, &mut entry.transform))
        })
    }

    /// Runs [`Script::update`] for each running object, one at a time, as a module does when its
    /// script has no [`Script::update_many`] of its own.
    pub fn update_each(&mut self, dt: f32) {
        self.give_back();
        if self.from_frame {
            #[cfg(target_arch = "wasm32")]
            for (slot, script) in self.slots.iter_mut().enumerate() {
                if let Some(script) = script
                    && crate::batch::select(slot as i32)
                {
                    script.update(dt);
                }
            }
        } else if let Some(script) = self.slots.first_mut().and_then(Option::as_mut) {
            script.update(dt);
        }
    }

    /// The entries to give items out of: the frame's, or the one object's from the host.
    fn take(&self) -> Option<Vec<Entry>> {
        if !self.from_frame {
            return Some(vec![Entry { transform: Transform::get(), flags: RUNNING, ..Entry::EMPTY }]);
        }
        #[cfg(target_arch = "wasm32")]
        {
            crate::batch::take_entries()
        }
        #[cfg(not(target_arch = "wasm32"))]
        None
    }

    /// The entries back where they came from: in the frame, or the one object's transform set
    /// through the host when it was given out.
    fn give_back(&mut self) {
        let Some(entries) = self.entries.take() else { return };
        if self.from_frame {
            #[cfg(target_arch = "wasm32")]
            crate::batch::give_back(entries);
        } else if let Some(entry) = entries.first()
            && entry.flags & CHANGED != 0
        {
            entry.transform.set();
        }
    }
}

impl<T: Script> Drop for Many<'_, T> {
    fn drop(&mut self) {
        self.give_back();
    }
}

/// Selects `slot` for one of `script`'s functions (used by [`crate::script!`]): false when it does
/// not run. The script reads its properties first when they may have changed.
#[doc(hidden)]
#[cfg(target_arch = "wasm32")]
pub fn ready<T: Script>(script: &mut T, slot: i32) -> bool {
    if !crate::batch::select(slot) {
        return false;
    }
    if usize::try_from(slot).is_ok_and(crate::batch::take_due) {
        script.read_properties();
    }
    true
}
