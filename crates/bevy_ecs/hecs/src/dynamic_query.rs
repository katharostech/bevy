use alloc::sync::Arc;
use bevy_utils::HashMap;
use core::{
    alloc::Layout,
    any::TypeId,
    hash::Hash,
    mem::{align_of, size_of},
    ptr::{slice_from_raw_parts, slice_from_raw_parts_mut, NonNull},
};
use std::{sync::Mutex, vec::Vec};

use crate::{archetype::ComponentIdSet, Access, Archetype, ComponentId, Entity, TypeInfo};

pub struct DynamicComponentInfoRegistry {
    /// Collection of dynamic component info that is kept to ensure the consistency of the info for
    /// dynamically created components.
    dynamic_components: Arc<Mutex<HashMap<u64, DynamicComponentInfo>>>,
}

impl DynamicComponentInfoRegistry {
    pub(crate) fn new() -> Self {
        Self {
            dynamic_components: Default::default(),
        }
    }
}

impl DynamicComponentInfoRegistry {
    /// Get the component info for the given Rust type
    pub fn get_rust_component_info<T: 'static>(&self) -> DynamicComponentInfo {
        DynamicComponentInfo {
            id: TypeId::of::<T>().into(),
            layout: Layout::from_size_align(size_of::<T>(), align_of::<T>()).unwrap(),
            drop: None,
        }
    }

    /// Registers a dynamic component and returns [`DynamicComponentInfo`] which can be used to
    /// spawn dynamic components with an [`EntityBuilder`].
    ///
    /// Returns `None` if a component with the provided ID has already been registered.
    pub fn register_dynamic_component(
        &self,
        external_id: u64,
        layout: Layout,
        drop: Option<fn(*mut u8)>,
    ) -> Option<DynamicComponentInfo> {
        let mut dynamic_components = self.dynamic_components.lock().unwrap();

        // If the component has already been registered
        if dynamic_components.contains_key(&external_id) {
            // Return none, we can't re-register the same component
            None

        // If the component has not been registered yet
        } else {
            // Register the component and return its info
            let info = DynamicComponentInfo {
                id: ComponentId::ExternalId(external_id),
                layout,
                drop,
            };
            dynamic_components.insert(external_id, info);

            Some(info)
        }
    }

    /// Return the component info for a non-Rust dynamic component. If the dynamic component has not
    /// been registered yet, it will return None.
    pub fn get_dynamic_component_info(&self, external_id: u64) -> Option<DynamicComponentInfo> {
        self.dynamic_components
            .lock()
            .unwrap()
            .get(&external_id)
            .map(Clone::clone)
    }
}

lazy_static::lazy_static! {
    /// The global registry of component info
    pub static ref DYNAMIC_COMPONENT_INFO_REGISTRY: DynamicComponentInfoRegistry =
        DynamicComponentInfoRegistry::new();
}

/// A Query that can be constructed at runtime
#[derive(Default)]
pub struct DynamicQuery {
    /// Whether or not the entity should be queried
    pub entity: bool,
    // fields private and then checking the values in the constructor.
    /// The list of accesses to immutable components
    pub immutable: Vec<DynamicComponentInfo>,
    /// The list of accesses to mutable components
    pub mutable: Vec<DynamicComponentInfo>,
}

impl DynamicQuery {
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
    // FIXME: Figure out if we need this
    pub(crate) fn _borrow(&self, archetype: &Archetype) {
        for component in self.immutable.iter() {
            archetype.borrow_component(component.id);
        }

        for component in self.mutable.iter() {
            archetype.borrow_component(component.id);
        }
    }

    /// Release dynamic borrow of the given archetype
    // FIXME: Figure out if we need this
    pub(crate) fn _release(&self, archetype: &Archetype) {
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

        // Create new fetch
        let mut fetch = DynamicQueryFetch::new(self);

        let mut matches_any = false;
        let mut missing_any = false;

        // Query the entity if requested
        if self.entity {
            fetch.entity = Some(archetype.entities())
        }

        // Query immutable components
        for component in &self.immutable {
            let ptr = archetype.get_dynamic(component.id, component.layout.size(), 0);

            if let Some(ptr) = ptr {
                matches_any = true;

                fetch
                    .immutable
                    .push(NonNull::new_unchecked(ptr.as_ptr().add(offset)));
            } else {
                missing_any = true;
            }
        }

        // Query mutable components
        for component in &self.mutable {
            let ptr = archetype.get_dynamic(component.id, component.layout.size(), 0);

            if let Some(ptr) = ptr {
                matches_any = true;

                fetch
                    .mutable
                    .push(NonNull::new_unchecked(ptr.as_ptr().add(offset)));
            } else {
                missing_any = true;
            }
        }

        if matches_any && !missing_any {
            Some(fetch)
        } else {
            None
        }
    }
}

pub struct DynamicQueryFetch<'query> {
    query: &'query DynamicQuery,
    entity: Option<NonNull<Entity>>,
    immutable: Vec<NonNull<u8>>,
    mutable: Vec<NonNull<u8>>,
}

impl<'query> DynamicQueryFetch<'query> {
    fn new(query: &'query DynamicQuery) -> Self {
        Self {
            query,
            entity: Default::default(),
            immutable: Default::default(),
            mutable: Default::default(),
        }
    }

    /// # Safety
    ///
    /// Index must be within bounds
    pub(crate) unsafe fn fetch<'a>(&mut self, index: usize) -> DynamicQueryResult<'a> {
        let mut query_result = DynamicQueryResult {
            // FIXME: Does this entity need to be cloned to be safe, or is it safe just to
            // dereference it?
            entity: self.entity.map(|x| *x.as_ptr().add(index)),
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
#[derive(Default)]
pub struct DynamicQueryResult<'a> {
    /// The entity, if requested in the Fetch
    pub entity: Option<Entity>,
    /// The immutable data for the immutable components requested
    pub immutable: Vec<&'a [u8]>,
    /// The mutable data for the immutable components requested
    pub mutable: Vec<&'a mut [u8]>,
}

/// Information about a dynamic component
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DynamicComponentInfo {
    /// The component ID
    id: ComponentId,
    /// The memory layout of the component
    layout: Layout,
    /// The function to call when the component is dropped from storage
    drop: Option<fn(*mut u8)>,
}

#[allow(clippy::derive_hash_xor_eq)] // Fine because we maintain k1 == k2 ⇒ hash(k1) == hash(k2)
impl Hash for DynamicComponentInfo {
    fn hash<H: core::hash::Hasher>(&self, state: &mut H) {
        self.id.hash(state);
        self.drop.hash(state);
        state.write_usize(self.layout.size());
        state.write_usize(self.layout.align());
    }
}

impl Into<TypeInfo> for DynamicComponentInfo {
    fn into(self) -> TypeInfo {
        let Self { id, layout, drop } = self;
        TypeInfo {
            id,
            layout,
            drop: drop.unwrap_or(|_| ()),
        }
    }
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
    iter: Option<DynamicChunkIter<'query>>,
}

impl<'borrow, 'query, 'world> Iterator for DynamicQueryIter<'borrow, 'query, 'world> {
    type Item = DynamicQueryResult<'world>;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        loop {
            match self.iter {
                None => {
                    let archetype = self.borrow.archetypes.get(self.archetype_index)?;
                    self.archetype_index += 1;
                    unsafe {
                        self.iter = self.borrow.query.get_fetch(archetype, 0).map(|fetch| {
                            DynamicChunkIter {
                                fetch,
                                len: archetype.len(),
                                position: 0,
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

pub struct DynamicChunkIter<'query> {
    fetch: DynamicQueryFetch<'query>,
    position: usize,
    len: usize,
}

impl<'query> DynamicChunkIter<'query> {
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

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn dynamic_component_info_registry_smoke() {
        let reg = DynamicComponentInfoRegistry::new();

        // Create a external component ids
        const ID1: u64 = 242237625853274575;
        const ID2: u64 = 6820197023594215835;

        // Registry should not contain dynamic components yet
        assert_eq!(reg.get_dynamic_component_info(ID1), None);
        assert_eq!(reg.get_dynamic_component_info(ID2), None);

        // Registry will still return Rust component type info
        let i32_info: DynamicComponentInfo = DynamicComponentInfo {
            id: ComponentId::RustTypeId(TypeId::of::<i32>()),
            layout: Layout::from_size_align(size_of::<i32>(), align_of::<i32>()).unwrap(),
            drop: None,
        };
        assert_eq!(reg.get_rust_component_info::<i32>(), i32_info);

        // Register a component
        let layout1 = Layout::from_size_align(16, 4).unwrap();
        let info1 = reg.register_dynamic_component(ID1, layout1, None);

        // Make sure component info is registered
        assert_eq!(
            info1,
            Some(DynamicComponentInfo {
                id: ComponentId::ExternalId(ID1),
                layout: layout1,
                drop: None,
            })
        );

        // Register another component
        let layout2 = Layout::from_size_align(4, 1).unwrap();
        let drop2 = |_| std::print!("test");
        let info2 = reg.register_dynamic_component(ID2, layout2, Some(drop2));

        // Make sure component info is registered
        assert_eq!(
            info2,
            Some(DynamicComponentInfo {
                id: ComponentId::ExternalId(ID2),
                layout: layout2,
                drop: Some(drop2),
            })
        );

        // Attempt to double-register id 1 with different information
        let info3 = reg.register_dynamic_component(ID1, layout2, None);
        assert_eq!(info3, None);

        // Query registered component info
        assert_eq!(info1, reg.get_dynamic_component_info(ID1));
        assert_eq!(info2, reg.get_dynamic_component_info(ID2));
    }

    #[test]
    #[should_panic(expected = "Found multiples of the same component in a DynamicQuery")]
    fn invalid_query_panics() {
        let reg = DynamicComponentInfoRegistry::new();

        // Create a external component ids
        const ID1: u64 = 242237625853274575;
        const ID2: u64 = 6820197023594215835;

        // Register components
        let layout1 = Layout::from_size_align(16, 4).unwrap();
        let info1 = reg.register_dynamic_component(ID1, layout1, None).unwrap();
        let layout2 = Layout::from_size_align(4, 1).unwrap();
        let info2 = reg.register_dynamic_component(ID2, layout2, None).unwrap();

        let mut query = DynamicQuery::default();

        // Add immutable query for info1 ( fine )
        query.immutable.push(info1);
        // Add mutable query for info2 ( fine )
        query.mutable.push(info2);
        // Add mutable query for info1 ( not fine, already in immutable query )
        query.mutable.push(info1);

        let archetype = Archetype::new(std::vec![]);

        // Getting the fetch for the query should panic because of the double borrow indicated in
        // the query
        unsafe { query.get_fetch(&archetype, 0) };
    }
}
