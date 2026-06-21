use std::mem::MaybeUninit;

const NIL: u32 = u32::MAX;

struct ArenaSlot<T> {
    data: MaybeUninit<T>,
    next_free: u32,
}

pub struct Arena<T> {
    slots: Vec<ArenaSlot<T>>,
    free_head: u32,
    len: u32,
}

impl<T> Arena<T> {
    pub fn new(capacity: usize) -> Self {
        let mut slots = Vec::with_capacity(capacity);
        for i in 0..capacity {
            slots.push(ArenaSlot {
                data: MaybeUninit::uninit(),
                next_free: if i + 1 < capacity { (i + 1) as u32 } else { NIL },
            });
        }
        Arena {
            slots,
            free_head: if capacity > 0 { 0 } else { NIL },
            len: 0,
        }
    }

    pub fn alloc(&mut self, value: T) -> Option<u32> {
        if self.free_head == NIL {
            return None;
        }
        let idx = self.free_head;
        let slot = &mut self.slots[idx as usize];
        self.free_head = slot.next_free;
        slot.data.write(value);
        slot.next_free = NIL;
        self.len += 1;
        Some(idx)
    }

    pub fn dealloc(&mut self, idx: u32) {
        debug_assert!((idx as usize) < self.slots.len());
        let slot = &mut self.slots[idx as usize];
        unsafe {
            slot.data.assume_init_drop();
        }
        slot.next_free = self.free_head;
        self.free_head = idx;
        self.len -= 1;
    }

    #[inline(always)]
    pub fn get(&self, idx: u32) -> &T {
        unsafe { self.slots[idx as usize].data.assume_init_ref() }
    }

    #[inline(always)]
    pub fn get_mut(&mut self, idx: u32) -> &mut T {
        unsafe { self.slots[idx as usize].data.assume_init_mut() }
    }

    #[inline(always)]
    pub fn len(&self) -> u32 {
        self.len
    }

    #[inline(always)]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}

impl<T> Drop for Arena<T> {
    fn drop(&mut self) {
        if std::mem::needs_drop::<T>() {
            let mut cur = self.free_head;
            let mut freed = vec![false; self.slots.len()];
            while cur != NIL {
                freed[cur as usize] = true;
                cur = self.slots[cur as usize].next_free;
            }
            for (i, slot) in self.slots.iter_mut().enumerate() {
                if !freed[i] {
                    unsafe {
                        slot.data.assume_init_drop();
                    }
                }
            }
        }
    }
}

unsafe impl<T: Send> Send for Arena<T> {}
unsafe impl<T: Sync> Sync for Arena<T> {}
