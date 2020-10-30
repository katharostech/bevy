use core::{
    alloc::Layout,
    ptr::{slice_from_raw_parts, slice_from_raw_parts_mut, NonNull},
};
use std::vec::Vec;

use crate::{archetype::ComponentIdSet, Access, Archetype, ComponentId, Entity, TypeInfo};

/// Provides the necessary type information about a dynamically registered component
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DynamicComponentInfo {
    /// The external component ID
    pub id: u64,
    /// The memory layout of the component
    pub layout: Layout,
    /// The function to call when the component is dropped from storage
    pub drop: fn(*mut u8),
}

impl Into<TypeInfo> for DynamicComponentInfo {
    fn into(self) -> TypeInfo {
        let Self { id, layout, drop } = self;
        TypeInfo {
            id: ComponentId::ExternalId(id),
            layout,
            drop,
        }
    }
}

/// A Query that can be constructed at runtime
#[derive(Default)]
pub struct DynamicQuery {
    /// The list of accesses to immutable components
    immutable: Vec<TypeInfo>,
    /// The list of accesses to mutable components
    mutable: Vec<TypeInfo>,
}

impl DynamicQuery {
    /// Add an immutable query to a rust component type
    ///
    /// # Panics
    /// Panics if the component has already been added to the query.
    pub fn push_rust<T: 'static>(&mut self) {
        let info = TypeInfo::of::<T>();
        if self.info_collides(&info) {
            panic!("Component already added to query: {:?}", info);
        }
        self.immutable.push(info);
    }

    /// Add a mutable query to a Rust component type
    ///
    /// # Panics
    /// Panics if the component has already been added to the query.
    pub fn push_rust_mut<T: 'static>(&mut self) {
        let info = TypeInfo::of::<T>();
        if self.info_collides(&info) {
            panic!("Component already added to query: {:?}", info);
        }
        self.mutable.push(info);
    }

    /// Add an immutable query to a dynamic external type
    ///
    /// # Panics
    /// Panics if the component has already been added to the query.
    pub fn push_dynamic(&mut self, info: DynamicComponentInfo) {
        let info: TypeInfo = info.into();
        if self.info_collides(&info) {
            panic!("Component already added to query: {:?}", info);
        }
        self.immutable.push(info);
    }

    /// Add a mutable query to a dynamic external type
    ///
    /// # Panics
    /// Panics if the component has already been added to the query.
    pub fn push_dynamic_mut(&mut self, info: DynamicComponentInfo) {
        let info: TypeInfo = info.into();
        if self.info_collides(&info) {
            panic!("Component already added to query: {:?}", info);
        }
        self.mutable.push(info);
    }

    /// Add an immutable query given the raw type info
    ///
    /// # Panics
    /// Panics if the component has already been added to the query.
    pub fn push_type_info(&mut self, info: TypeInfo) {
        if self.info_collides(&info) {
            panic!("Component already added to query: {:?}", info);
        }
        self.immutable.push(info);
    }

    /// Add a mutable query given the raw type info
    ///
    /// # Panics
    /// Panics if the component has already been added to the query.
    pub fn push_type_info_mut(&mut self, info: TypeInfo) {
        if self.info_collides(&info) {
            panic!("Component already added to query: {:?}", info);
        }
        self.mutable.push(info);
    }

    /// Returns true if the given info collides with the info already present
    fn info_collides(&self, info: &TypeInfo) -> bool {
        for item in &self.immutable {
            if item.id() == info.id() {
                return true;
            }
        }
        for item in &self.mutable {
            if item.id() == info.id() {
                return true;
            }
        }

        false
    }

    /// Returns how, if at all, the query accesses the given archetype
    pub fn access(&self, archetype: &Archetype) -> Option<Access> {
        let mut access = None;

        for component in self.immutable.iter() {
            if archetype.has_component(component.id) {
                access = Some(Access::Read);
            }
        }

        for component in self.mutable.iter() {
            if archetype.has_component(component.id) {
                access = Some(Access::Write);
            }
        }

        access
    }

    /// Acquires a dynamic borrow to the archetype
    pub(crate) fn borrow(&self, archetype: &Archetype) {
        for component in self.immutable.iter() {
            archetype.borrow_component(component.id);
        }

        for component in self.mutable.iter() {
            archetype.borrow_component(component.id);
        }
    }

    /// Release dynamic borrow of the given archetype
    pub(crate) fn release(&self, archetype: &Archetype) {
        for component in self.immutable.iter() {
            archetype.release_component(component.id);
        }

        for component in self.mutable.iter() {
            archetype.release_component(component.id);
        }
    }

    /// Indicate whether or not the given item should be skipped
    pub(crate) fn should_skip(&self, _item_index: usize) -> bool {
        // TODO: This would be used for changed component notifications, but we don't support that
        // in dynamic queries yet
        false
    }

    /// # Safety
    /// `offset` must be in bounds of `archetype`
    /// # Panics
    /// This function will panic if there is an overlap between the components in the mutable and
    /// immutable query lists.
    pub(crate) unsafe fn get_fetch<'query>(
        &'query self,
        archetype: &Archetype,
        offset: usize,
    ) -> Option<DynamicQueryFetch> {
        // Validate no overlap between mutable and immutable queries
        let mut ids = ComponentIdSet::default();
        for component in &self.immutable {
            if !ids.insert(component.id) {
                panic!("Found multiples of the same component in a DynamicQuery");
            }
        }
        for component in &self.mutable {
            if !ids.insert(component.id) {
                panic!("Found multiples of the same component in a DynamicQuery");
            }
        }

        let mut matches_any = false;
        let mut missing_any = false;

        // Query immutable components
        let mut immutable = Vec::with_capacity(self.immutable.len());
        for component in &self.immutable {
            let ptr = archetype.get_dynamic(component.id, component.layout.size(), 0);

            if let Some(ptr) = ptr {
                matches_any = true;

                immutable.push(NonNull::new_unchecked(ptr.as_ptr().add(offset)));
            } else {
                missing_any = true;
            }
        }

        // Query mutable components
        let mut mutable = Vec::with_capacity(self.mutable.len());
        for component in &self.mutable {
            let ptr = archetype.get_dynamic(component.id, component.layout.size(), 0);

            if let Some(ptr) = ptr {
                matches_any = true;

                mutable.push(NonNull::new_unchecked(ptr.as_ptr().add(offset)));
            } else {
                missing_any = true;
            }
        }

        if matches_any && !missing_any {
            Some(DynamicQueryFetch {
                query: self,
                entity: archetype.entities(),
                mutable,
                immutable,
            })
        } else {
            None
        }
    }
}

pub struct DynamicQueryFetch<'query> {
    query: &'query DynamicQuery,
    entity: NonNull<Entity>,
    immutable: Vec<NonNull<u8>>,
    mutable: Vec<NonNull<u8>>,
}

impl<'query> DynamicQueryFetch<'query> {
    /// # Safety
    ///
    /// Index must be within bounds
    pub(crate) unsafe fn fetch<'a>(&mut self, index: usize) -> DynamicQueryResult<'a> {
        let mut query_result = DynamicQueryResult {
            // FIXME: Does this entity need to be cloned to be safe, or is it safe just to
            // dereference it?
            entity: *self.entity.as_ptr().add(index),
            immutable: Vec::with_capacity(self.immutable.len()),
            mutable: Vec::with_capacity(self.mutable.len()),
        };

        for (component, ptr) in self.query.immutable.iter().zip(&self.immutable) {
            query_result.immutable.push(&*slice_from_raw_parts(
                ptr.as_ptr().add(index * component.layout.size()),
                component.layout.size(),
            ));
        }

        for (component, ptr) in self.query.mutable.iter().zip(&self.mutable) {
            query_result.mutable.push(&mut *slice_from_raw_parts_mut(
                ptr.as_ptr().add(index * component.layout.size()),
                component.layout.size(),
            ));
        }

        query_result
    }
}

/// The data returned by [`DynamicFetch::fetch`]
pub struct DynamicQueryResult<'a> {
    /// The entity, if requested in the Fetch
    pub entity: Entity,
    // TODO: Find out if there is a way to make these borrowed from something instead of requring
    // owned `Vec`'s
    /// The immutable data for the immutable components requested
    pub immutable: Vec<&'a [u8]>,
    /// The mutable data for the immutable components requested
    pub mutable: Vec<&'a mut [u8]>,
}

/// A borrow capable of executing a dynamic query on the world
pub struct DynamicQueryBorrow<'query, 'world> {
    query: &'query DynamicQuery,
    archetypes: &'world [Archetype],
    borrowed: bool,
}

impl<'query, 'world> DynamicQueryBorrow<'query, 'world> {
    /// Create a borrow for the provided query on the given world which can be used to execute the
    /// query.
    pub fn new(archetypes: &'world [Archetype], query: &'query DynamicQuery) -> Self {
        Self {
            archetypes,
            query,
            borrowed: false,
        }
    }

    /// Create an iterator over the query
    pub fn iter_mut<'borrow>(&'borrow mut self) -> DynamicQueryIter<'borrow, 'query, 'world> {
        if self.borrowed {
            panic!("call iter_mut on query multiple times");
        }

        self.borrowed = true;
        DynamicQueryIter {
            borrow: self,
            archetype_index: 0,
            iter: None,
        }
    }
}

impl<'borrow, 'query, 'world> IntoIterator for &'borrow mut DynamicQueryBorrow<'query, 'world> {
    type IntoIter = DynamicQueryIter<'borrow, 'query, 'world>;
    type Item = DynamicQueryResult<'world>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter_mut()
    }
}

pub struct DynamicQueryIter<'borrow, 'query, 'world> {
    borrow: &'borrow mut DynamicQueryBorrow<'query, 'world>,
    archetype_index: usize,
    iter: Option<DynamicChunkIter<'query, 'world>>,
}

impl<'borrow, 'query, 'world> Iterator for DynamicQueryIter<'borrow, 'query, 'world> {
    type Item = DynamicQueryResult<'world>;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        loop {
            match self.iter {
                None => {
                    let archetype = self.borrow.archetypes.get(self.archetype_index)?;
                    self.borrow.query.borrow(archetype);
                    self.archetype_index += 1;
                    unsafe {
                        self.iter = self.borrow.query.get_fetch(archetype, 0).map(|fetch| {
                            DynamicChunkIter {
                                fetch,
                                len: archetype.len(),
                                position: 0,
                                archetype,
                            }
                        });
                    }
                }
                Some(ref mut iter) => match unsafe { iter.next() } {
                    None => {
                        self.iter = None;
                        continue;
                    }
                    Some(components) => {
                        return Some(components);
                    }
                },
            }
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let n = self.len();
        (n, Some(n))
    }
}

impl<'borrow, 'query, 'world> ExactSizeIterator for DynamicQueryIter<'borrow, 'query, 'world> {
    fn len(&self) -> usize {
        self.borrow
            .archetypes
            .iter()
            .filter(|&x| self.borrow.query.access(x).is_some())
            .map(|x| x.len())
            .sum()
    }
}

pub struct DynamicChunkIter<'query, 'world> {
    fetch: DynamicQueryFetch<'query>,
    position: usize,
    len: usize,
    archetype: &'world Archetype,
}

impl<'query, 'world> DynamicChunkIter<'query, 'world> {
    unsafe fn next<'a>(&mut self) -> Option<DynamicQueryResult<'a>> {
        loop {
            if self.position == self.len {
                return None;
            }

            if self.fetch.query.should_skip(self.position as usize) {
                self.position += 1;
                continue;
            }

            let item = Some(self.fetch.fetch(self.position as usize));

            self.position += 1;
            return item;
        }
    }
}

impl<'query, 'world> Drop for DynamicChunkIter<'query, 'world> {
    fn drop(&mut self) {
        self.fetch.query.release(self.archetype);
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    #[should_panic(expected = "Component already added to query")]
    fn invalid_query_panics() {
        // Create a external component ids
        const ID1: u64 = 242237625853274575;
        const ID2: u64 = 6820197023594215835;

        // Register components
        let layout1 = Layout::from_size_align(16, 4).unwrap();
        let layout2 = Layout::from_size_align(4, 1).unwrap();

        let mut query = DynamicQuery::default();

        let info1 = DynamicComponentInfo {
            id: ID1,
            layout: layout1,
            drop: |_| (),
        };
        let info2 = DynamicComponentInfo {
            id: ID2,
            layout: layout2,
            drop: |_| (),
        };

        // Add immutable query for info1 ( fine )
        query.push_dynamic(info1);
        // Add mutable query for info2 ( fine )
        query.push_dynamic_mut(info2);
        // Add mutable query for info1 ( not fine, already in immutable query )
        query.push_dynamic_mut(info1);

        let archetype = Archetype::new(std::vec![]);

        // Getting the fetch for the query should panic because of the double borrow indicated in
        // the query
        unsafe { query.get_fetch(&archetype, 0) };
    }
}
