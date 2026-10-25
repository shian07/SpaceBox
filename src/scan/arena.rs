use std::fmt;

pub const NONE: u32 = u32::MAX;
const NAME_CHUNK: usize = 64 * 1024;

pub mod flags {
    pub const DIR: u16 = 1 << 0;
    pub const SMALL_AGG: u16 = 1 << 1;
    pub const OTHER_FS: u16 = 1 << 2;
    pub const SYMLINK: u16 = 1 << 3;
    pub const ROOT: u16 = 1 << 4;
    /// Category of a file, 0..=8. Bits 8..=11.
    pub const CAT_SHIFT: u16 = 8;
    pub const CAT_MASK: u16 = 0xF << CAT_SHIFT;
    /// Dominant category of a directory's contents, 0..=8, `DOMINANT_NONE` if
    /// no category holds a meaningful share. Bits 12..=15.
    ///
    /// Stored in spare flag bits rather than new node fields: `Node` is 40
    /// bytes and a per-category byte tally would multiply that by nine, which
    /// the 20 MiB RSS budget cannot absorb on large trees.
    pub const DOM_SHIFT: u16 = 12;
    pub const DOM_MASK: u16 = 0xF << DOM_SHIFT;
    /// No dominant category (draw the folder in neutral grey).
    pub const DOMINANT_NONE: u8 = 0xF;
}

#[derive(Clone, Copy)]
pub struct Node {
    pub parent: u32,
    pub first: u32,
    pub next: u32,
    pub name_off: u32,
    pub name_len: u16,
    pub flags: u16,
    pub alloc: u64,
    pub logical: u64,
    pub files: u32,
}

impl Node {
    pub fn is_dir(&self) -> bool {
        self.flags & flags::DIR != 0
    }

    pub fn is_small_agg(&self) -> bool {
        self.flags & flags::SMALL_AGG != 0
    }

    pub fn is_other_fs(&self) -> bool {
        self.flags & flags::OTHER_FS != 0
    }

    pub fn is_symlink(&self) -> bool {
        self.flags & flags::SYMLINK != 0
    }

    pub fn category(&self) -> u8 {
        ((self.flags & flags::CAT_MASK) >> flags::CAT_SHIFT) as u8
    }

    /// The dominant content category of a directory, if one was recorded.
    pub fn dominant_category(&self) -> Option<u8> {
        let value = ((self.flags & flags::DOM_MASK) >> flags::DOM_SHIFT) as u8;
        (value != flags::DOMINANT_NONE).then_some(value)
    }

    pub fn set_dominant_category(&mut self, category: u8) {
        let value = category.min(flags::DOMINANT_NONE);
        self.flags = (self.flags & !flags::DOM_MASK) | ((value as u16) << flags::DOM_SHIFT);
    }
}

impl fmt::Debug for Node {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Node")
            .field("parent", &self.parent)
            .field("first", &self.first)
            .field("next", &self.next)
            .field("flags", &self.flags)
            .field("alloc", &self.alloc)
            .field("logical", &self.logical)
            .field("files", &self.files)
            .finish()
    }
}

#[derive(Default)]
pub struct NameArena {
    chunks: Vec<Box<[u8]>>,
    len: u32,
}

impl NameArena {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn intern(&mut self, name: &str) -> (u32, u16) {
        let bytes = name.as_bytes();
        let n = bytes.len().min(u16::MAX as usize);
        if n == 0 {
            return (self.len, 0);
        }

        let pos = self.len as usize % NAME_CHUNK;
        if pos + n > NAME_CHUNK {
            self.len = self.len.saturating_add((NAME_CHUNK - pos) as u32);
        }

        let chunk_index = self.len as usize / NAME_CHUNK;
        while self.chunks.len() <= chunk_index {
            self.chunks.push(vec![0u8; NAME_CHUNK].into_boxed_slice());
        }

        let start = self.len as usize % NAME_CHUNK;
        self.chunks[chunk_index][start..start + n].copy_from_slice(&bytes[..n]);

        let offset = self.len;
        self.len = self.len.saturating_add(n as u32);
        (offset, n as u16)
    }

    pub fn get(&self, offset: u32, len: u16) -> &str {
        if len == 0 {
            return "";
        }
        let start = offset as usize % NAME_CHUNK;
        let end = start + len as usize;
        let Some(chunk) = self.chunks.get(offset as usize / NAME_CHUNK) else {
            return "?";
        };
        if end > NAME_CHUNK {
            return "?";
        }
        std::str::from_utf8(&chunk[start..end]).unwrap_or("?")
    }

    #[cfg(test)]
    pub fn bytes_used(&self) -> usize {
        self.chunks.len() * NAME_CHUNK
    }
}

pub struct TreeArena {
    pub nodes: Vec<Node>,
    pub names: NameArena,
}

pub struct ChildIter<'a> {
    arena: &'a TreeArena,
    next: u32,
}

impl Iterator for ChildIter<'_> {
    type Item = u32;

    fn next(&mut self) -> Option<Self::Item> {
        if self.next == NONE {
            return None;
        }
        let id = self.next;
        self.next = self.arena.nodes[id as usize].next;
        Some(id)
    }
}

impl TreeArena {
    pub fn new() -> Self {
        Self {
            nodes: Vec::new(),
            names: NameArena::new(),
        }
    }

    pub fn add_node(
        &mut self,
        parent: u32,
        name: &str,
        node_flags: u16,
        alloc: u64,
        logical: u64,
    ) -> u32 {
        let (name_off, name_len) = self.names.intern(name);
        let id = self.nodes.len() as u32;
        let next = if parent == NONE {
            NONE
        } else {
            self.nodes[parent as usize].first
        };
        self.nodes.push(Node {
            parent,
            first: NONE,
            next,
            name_off,
            name_len,
            flags: node_flags,
            alloc,
            logical,
            files: 0,
        });
        if parent != NONE {
            self.nodes[parent as usize].first = id;
        }
        id
    }

    pub fn name(&self, id: u32) -> &str {
        let node = &self.nodes[id as usize];
        self.names.get(node.name_off, node.name_len)
    }

    pub fn children(&self, id: u32) -> ChildIter<'_> {
        ChildIter {
            arena: self,
            next: self.nodes[id as usize].first,
        }
    }

    pub fn parent(&self, id: u32) -> Option<u32> {
        let parent = self.nodes[id as usize].parent;
        (parent != NONE).then_some(parent)
    }

    pub fn ancestor_ids(&self, id: u32) -> Vec<u32> {
        let mut ids = Vec::with_capacity(16);
        let mut current = id;
        loop {
            ids.push(current);
            let parent = self.nodes[current as usize].parent;
            if parent == NONE {
                break;
            }
            current = parent;
        }
        ids.reverse();
        ids
    }

    #[cfg(test)]
    pub fn memory_bytes(&self) -> usize {
        self.nodes.capacity() * std::mem::size_of::<Node>() + self.names.bytes_used()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_survive_chunk_boundaries() {
        let mut arena = NameArena::new();
        let long = "x".repeat(60_000);
        let crossing = "y".repeat(10_000);
        let (off1, len1) = arena.intern(&long);
        let (off2, len2) = arena.intern(&crossing);
        assert_eq!(arena.get(off1, len1), long);
        assert_eq!(arena.get(off2, len2), crossing);
        assert!(arena.bytes_used() >= 2 * NAME_CHUNK);
    }

    #[test]
    fn builds_linked_children() {
        let mut arena = TreeArena::new();
        let root = arena.add_node(NONE, "root", flags::ROOT | flags::DIR, 0, 0);
        let a = arena.add_node(root, "a", flags::DIR, 10, 10);
        let b = arena.add_node(root, "b", 0, 20, 20);
        let children: Vec<u32> = arena.children(root).collect();
        assert_eq!(children.len(), 2);
        assert!(children.contains(&a));
        assert!(children.contains(&b));
        assert_eq!(arena.ancestor_ids(b), vec![root, b]);
    }
}
