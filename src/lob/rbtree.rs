use crate::lob::arena::Arena;

pub const NIL: u32 = u32::MAX;

pub const COLOR_BLACK: u8 = 0;
pub const COLOR_RED: u8 = 1;

#[derive(Clone, Copy)]
pub struct RbNode {
    pub key: i64,
    pub color: u8,
    pub left: u32,
    pub right: u32,
    pub parent: u32,
    pub order_head: u32,
    pub order_tail: u32,
    pub total_qty: u64,
}

impl RbNode {
    pub fn new(key: i64) -> Self {
        RbNode {
            key,
            color: COLOR_RED,
            left: NIL,
            right: NIL,
            parent: NIL,
            order_head: NIL,
            order_tail: NIL,
            total_qty: 0,
        }
    }
}

pub struct RbTree {
    pub root: u32,
    pub arena: Arena<RbNode>,
}

impl RbTree {
    pub fn new(capacity: usize) -> Self {
        RbTree {
            root: NIL,
            arena: Arena::new(capacity),
        }
    }

    #[inline(always)]
    fn left_rotate(&mut self, x: u32) {
        let y = self.arena.get(x).right;
        debug_assert!(y != NIL);
        let y_left = self.arena.get(y).left;

        self.arena.get_mut(x).right = y_left;
        if y_left != NIL {
            self.arena.get_mut(y_left).parent = x;
        }

        let x_parent = self.arena.get(x).parent;
        self.arena.get_mut(y).parent = x_parent;
        if x_parent == NIL {
            self.root = y;
        } else if x == self.arena.get(x_parent).left {
            self.arena.get_mut(x_parent).left = y;
        } else {
            self.arena.get_mut(x_parent).right = y;
        }

        self.arena.get_mut(y).left = x;
        self.arena.get_mut(x).parent = y;
    }

    #[inline(always)]
    fn right_rotate(&mut self, y: u32) {
        let x = self.arena.get(y).left;
        debug_assert!(x != NIL);
        let x_right = self.arena.get(x).right;

        self.arena.get_mut(y).left = x_right;
        if x_right != NIL {
            self.arena.get_mut(x_right).parent = y;
        }

        let y_parent = self.arena.get(y).parent;
        self.arena.get_mut(x).parent = y_parent;
        if y_parent == NIL {
            self.root = x;
        } else if y == self.arena.get(y_parent).right {
            self.arena.get_mut(y_parent).right = x;
        } else {
            self.arena.get_mut(y_parent).left = x;
        }

        self.arena.get_mut(x).right = y;
        self.arena.get_mut(y).parent = x;
    }

    fn insert_fixup(&mut self, mut z: u32) {
        while self.arena.get(z).parent != NIL
            && self.arena.get(self.arena.get(z).parent).color == COLOR_RED
        {
            let parent = self.arena.get(z).parent;
            let grandparent = self.arena.get(parent).parent;
            if grandparent == NIL {
                break;
            }
            if parent == self.arena.get(grandparent).left {
                let y = self.arena.get(grandparent).right;
                if y != NIL && self.arena.get(y).color == COLOR_RED {
                    self.arena.get_mut(parent).color = COLOR_BLACK;
                    self.arena.get_mut(y).color = COLOR_BLACK;
                    self.arena.get_mut(grandparent).color = COLOR_RED;
                    z = grandparent;
                } else {
                    if z == self.arena.get(parent).right {
                        z = parent;
                        self.left_rotate(z);
                    }
                    let z_parent = self.arena.get(z).parent;
                    let z_grandparent = self.arena.get(z_parent).parent;
                    self.arena.get_mut(z_parent).color = COLOR_BLACK;
                    if z_grandparent != NIL {
                        self.arena.get_mut(z_grandparent).color = COLOR_RED;
                    }
                    self.right_rotate(z_grandparent);
                }
            } else {
                let y = self.arena.get(grandparent).left;
                if y != NIL && self.arena.get(y).color == COLOR_RED {
                    self.arena.get_mut(parent).color = COLOR_BLACK;
                    self.arena.get_mut(y).color = COLOR_BLACK;
                    self.arena.get_mut(grandparent).color = COLOR_RED;
                    z = grandparent;
                } else {
                    if z == self.arena.get(parent).left {
                        z = parent;
                        self.right_rotate(z);
                    }
                    let z_parent = self.arena.get(z).parent;
                    let z_grandparent = self.arena.get(z_parent).parent;
                    self.arena.get_mut(z_parent).color = COLOR_BLACK;
                    if z_grandparent != NIL {
                        self.arena.get_mut(z_grandparent).color = COLOR_RED;
                    }
                    self.left_rotate(z_grandparent);
                }
            }
        }
        self.arena.get_mut(self.root).color = COLOR_BLACK;
    }

    pub fn insert(&mut self, key: i64) -> Option<u32> {
        let mut y: u32 = NIL;
        let mut x = self.root;
        while x != NIL {
            y = x;
            if key < self.arena.get(x).key {
                x = self.arena.get(x).left;
            } else if key > self.arena.get(x).key {
                x = self.arena.get(x).right;
            } else {
                return Some(x);
            }
        }

        let z = self.arena.alloc(RbNode::new(key))?;
        self.arena.get_mut(z).parent = y;
        if y == NIL {
            self.root = z;
        } else if key < self.arena.get(y).key {
            self.arena.get_mut(y).left = z;
        } else {
            self.arena.get_mut(y).right = z;
        }
        self.insert_fixup(z);
        Some(z)
    }

    fn transplant(&mut self, u: u32, v: u32) {
        let u_parent = self.arena.get(u).parent;
        if u_parent == NIL {
            self.root = v;
        } else if u == self.arena.get(u_parent).left {
            self.arena.get_mut(u_parent).left = v;
        } else {
            self.arena.get_mut(u_parent).right = v;
        }
        if v != NIL {
            self.arena.get_mut(v).parent = u_parent;
        }
    }

    fn minimum(&self, mut x: u32) -> u32 {
        while self.arena.get(x).left != NIL {
            x = self.arena.get(x).left;
        }
        x
    }

    fn maximum(&self, mut x: u32) -> u32 {
        while self.arena.get(x).right != NIL {
            x = self.arena.get(x).right;
        }
        x
    }

    fn delete_fixup(&mut self, mut x: u32) {
        while x != self.root && self.arena.get(x).color == COLOR_BLACK {
            let x_parent = self.arena.get(x).parent;
            if x_parent == NIL {
                break;
            }
            if x == self.arena.get(x_parent).left {
                let mut w = self.arena.get(x_parent).right;
                if w != NIL && self.arena.get(w).color == COLOR_RED {
                    self.arena.get_mut(w).color = COLOR_BLACK;
                    self.arena.get_mut(x_parent).color = COLOR_RED;
                    self.left_rotate(x_parent);
                    w = self.arena.get(x_parent).right;
                }
                if w != NIL {
                    let w_left = self.arena.get(w).left;
                    let w_right = self.arena.get(w).right;
                    let left_black = w_left == NIL || self.arena.get(w_left).color == COLOR_BLACK;
                    let right_black = w_right == NIL || self.arena.get(w_right).color == COLOR_BLACK;
                    if left_black && right_black {
                        self.arena.get_mut(w).color = COLOR_RED;
                        x = x_parent;
                    } else {
                        if right_black {
                            if w_left != NIL {
                                self.arena.get_mut(w_left).color = COLOR_BLACK;
                            }
                            self.arena.get_mut(w).color = COLOR_RED;
                            self.right_rotate(w);
                            w = self.arena.get(x_parent).right;
                        }
                        self.arena.get_mut(w).color = self.arena.get(x_parent).color;
                        self.arena.get_mut(x_parent).color = COLOR_BLACK;
                        let w_right = self.arena.get(w).right;
                        if w_right != NIL {
                            self.arena.get_mut(w_right).color = COLOR_BLACK;
                        }
                        self.left_rotate(x_parent);
                        x = self.root;
                    }
                } else {
                    break;
                }
            } else {
                let mut w = self.arena.get(x_parent).left;
                if w != NIL && self.arena.get(w).color == COLOR_RED {
                    self.arena.get_mut(w).color = COLOR_BLACK;
                    self.arena.get_mut(x_parent).color = COLOR_RED;
                    self.right_rotate(x_parent);
                    w = self.arena.get(x_parent).left;
                }
                if w != NIL {
                    let w_left = self.arena.get(w).left;
                    let w_right = self.arena.get(w).right;
                    let left_black = w_left == NIL || self.arena.get(w_left).color == COLOR_BLACK;
                    let right_black = w_right == NIL || self.arena.get(w_right).color == COLOR_BLACK;
                    if left_black && right_black {
                        self.arena.get_mut(w).color = COLOR_RED;
                        x = x_parent;
                    } else {
                        if left_black {
                            if w_right != NIL {
                                self.arena.get_mut(w_right).color = COLOR_BLACK;
                            }
                            self.arena.get_mut(w).color = COLOR_RED;
                            self.left_rotate(w);
                            w = self.arena.get(x_parent).left;
                        }
                        self.arena.get_mut(w).color = self.arena.get(x_parent).color;
                        self.arena.get_mut(x_parent).color = COLOR_BLACK;
                        let w_left = self.arena.get(w).left;
                        if w_left != NIL {
                            self.arena.get_mut(w_left).color = COLOR_BLACK;
                        }
                        self.right_rotate(x_parent);
                        x = self.root;
                    }
                } else {
                    break;
                }
            }
        }
        if x != NIL {
            self.arena.get_mut(x).color = COLOR_BLACK;
        }
    }

    pub fn delete(&mut self, z: u32) {
        debug_assert!(z != NIL);
        let z_left = self.arena.get(z).left;
        let z_right = self.arena.get(z).right;
        let mut y = z;
        let mut y_orig_color = self.arena.get(y).color;
        let x: u32;
        if z_left == NIL {
            x = z_right;
            self.transplant(z, z_right);
        } else if z_right == NIL {
            x = z_left;
            self.transplant(z, z_left);
        } else {
            y = self.minimum(z_right);
            y_orig_color = self.arena.get(y).color;
            x = self.arena.get(y).right;
            if self.arena.get(y).parent == z {
                if x != NIL {
                    self.arena.get_mut(x).parent = y;
                }
            } else {
                self.transplant(y, x);
                let z_right = self.arena.get(z).right;
                self.arena.get_mut(y).right = z_right;
                self.arena.get_mut(z_right).parent = y;
            }
            self.transplant(z, y);
            self.arena.get_mut(y).left = self.arena.get(z).left;
            self.arena.get_mut(z_left).parent = y;
            self.arena.get_mut(y).color = self.arena.get(z).color;
        }
        if y_orig_color == COLOR_BLACK {
            if x != NIL {
                self.delete_fixup(x);
            }
        }
        self.arena.dealloc(z);
    }

    pub fn find(&self, key: i64) -> Option<u32> {
        let mut x = self.root;
        while x != NIL {
            if key < self.arena.get(x).key {
                x = self.arena.get(x).left;
            } else if key > self.arena.get(x).key {
                x = self.arena.get(x).right;
            } else {
                return Some(x);
            }
        }
        None
    }

    pub fn min_key(&self) -> Option<u32> {
        if self.root == NIL {
            return None;
        }
        Some(self.minimum(self.root))
    }

    pub fn max_key(&self) -> Option<u32> {
        if self.root == NIL {
            return None;
        }
        Some(self.maximum(self.root))
    }

    pub fn is_empty(&self) -> bool {
        self.root == NIL
    }
}
