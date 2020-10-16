use crate::{alloc::vec::Vec, Entity, Location};
use bevy_utils::{HashMap, HashMapExt};
use std::{any::TypeId, fmt::Debug};

use crate::{borrow::AtomicBorrow, query::Fetch, Access, Component, Query};

/// Vector-based component storage backend
///
/// This is the default component storage backend used internally.
// TODO: Should this not be public?
pub struct VecComponentStorage<T> {
    /// The backing vector storage
    storage: Vec<T>,
    /// The storage metadata
    meta: ComponentStorageMeta,
}

impl<T> Default for VecComponentStorage<T> {
    /// Initialize empty storage
    fn default() -> Self {
        Self {
            storage: Vec::default(),
            meta: ComponentStorageMeta::default(),
        }
    }
}

impl<T> ComponentStorage for VecComponentStorage<T>
where
    T: 'static,
{
    fn meta(&self) -> &ComponentStorageMeta {
        &self.meta
    }

    fn meta_mut(&mut self) -> &mut ComponentStorageMeta {
        &mut self.meta
    }

    fn get_type(&self) -> TypeId {
        TypeId::of::<T>()
    }

    fn get_pointer(&self) -> *const u8 {
        self.storage.as_ptr().cast::<u8>()
    }

    fn get_value(&self, index: usize) -> *const u8 {
        unsafe { self.storage.get_unchecked(index) as *const T as *const u8 }
    }

    fn push(&mut self, value: *const u8) {
        unsafe {
            self.storage.push(value.cast::<T>().read());
        }
    }

    fn reserve(&mut self, count: usize) {
        self.storage.reserve(count);
    }

    fn swap_remove(&mut self, index: usize, forget: bool) {
        let value = self.storage.swap_remove(index);
        if forget {
            std::mem::forget(value);
        }
    }

    fn clear(&mut self) {
        self.storage.clear()
    }

    fn len(&self) -> usize {
        self.storage.len()
    }

    fn capacity(&self) -> usize {
        self.storage.capacity()
    }

    fn item_size(&self) -> usize {
        std::mem::size_of::<T>()
    }
}

/// Trait that allows a type to be used as a component storage backend
///
/// The current implementation used internally is [`VecComponentStorage`].
pub trait ComponentStorage {
    /// Return the component storages associated metadata
    fn meta(&self) -> &ComponentStorageMeta;
    /// Return a mutable reference to the component storage metadata
    fn meta_mut(&mut self) -> &mut ComponentStorageMeta;
    /// Get the [`TypeId`] that the storage is configured to store
    fn get_type(&self) -> TypeId;
    /// Get a pointer to the backing storage
    fn get_pointer(&self) -> *const u8;
    /// Get a pointer to the value at the given index
    fn get_value(&self, index: usize) -> *const u8;
    /// Request that enough space to fit `count` more elements be allocated in the backing storage
    ///
    /// The backing storage may decide to reserve more space than requested to avoid reallocations
    /// and it may not do anything if the backing storage already has enough room for the given
    /// number of extra items.
    fn reserve(&mut self, count: usize);
    /// Given a pointer to the value, add it to the end of the backing storage
    // TODO: Do we not want to make it required to add the item to the *end* of the storage? For now
    // that seems like a requirement.
    fn push(&mut self, value: *const u8);
    /// Get number of items in the storage
    fn len(&self) -> usize;
    /// Get the number of items that the storage has room for without re-allocating
    fn capacity(&self) -> usize;
    /// Remove the item at index and replace it with the last item in the storage
    ///
    /// If `forget` is true, the item will be forgotten and it's destructor will not be run.
    fn swap_remove(&mut self, index: usize, forget: bool);
    /// Clear the storage of all items, memory will not be de-allocated so it is available for
    /// future use
    fn clear(&mut self);
    /// Return the size of each item in the storage
    fn item_size(&self) -> usize;
}

/// A collection of entities having the same component types
///
/// Accessing `Archetype`s is only required for complex dynamic scheduling. To manipulate entities,
/// go through the `World`.
///
/// [`type_info`], [`entities`], and [`component_storages`] ( private ) are all kept in sync such
/// that an index into one of them will correspond to the same index in the others.
pub struct Archetype {
    /// Vector containing information about each of the component types that is stored in this
    /// archetype
    pub type_info: Vec<TypeInfo>,
    /// Vector of entities  stored in this archetype
    pub entities: Vec<Entity>,
    /// The mapping of the [`TypeId`] to the index in the [`type_info`], [`entities`], and
    /// [`component_storages`] ( private ) vectors
    pub type_indices: HashMap<TypeId, usize>,
    /// The number of entities to allocate room for every time we exceed our current capacity
    grow_size: usize,
    /// The component storages associated to each entity in [`entities`]
    component_storages: Vec<Box<dyn ComponentStorage>>,
}

impl std::fmt::Debug for Archetype {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Archetype")
            .field("type_info", &self.type_info)
            .field("entitites", &self.entities)
            .field("type_indices", &self.type_indices)
            .field("grow_size", &self.grow_size)
            .field("component_storages", &"Vec<Box<dyn ComponentStorage>>")
            .finish()
    }
}

impl Archetype {
    #[allow(missing_docs)]
    pub fn new(type_info: Vec<TypeInfo>) -> Self {
        Self::with_grow(type_info, 64)
    }

    #[allow(missing_docs)]
    pub fn with_grow(type_info: Vec<TypeInfo>, grow_size: usize) -> Self {
        // TODO: check for dupes
        let mut component_storages = Vec::with_capacity(type_info.len());
        let mut type_indices = HashMap::with_capacity(type_info.len());
        for ty in &type_info {
            type_indices.insert(ty.id, component_storages.len());
            component_storages.push((ty.get_storage)());
        }

        Self {
            type_info,
            entities: Vec::new(),
            component_storages,
            type_indices,
            grow_size,
        }
    }

    pub(crate) fn clear(&mut self) {
        for storage in self.component_storages.iter_mut() {
            storage.clear()
        }
        self.entities.clear();
    }

    #[allow(missing_docs)]
    #[inline]
    pub fn has<T: Component>(&self) -> bool {
        self.has_type(TypeId::of::<T>())
    }

    #[allow(missing_docs)]
    #[inline]
    pub fn has_type(&self, ty: TypeId) -> bool {
        self.type_indices.contains_key(&ty)
    }

    #[inline]
    fn type_index<T: Component>(&self) -> Option<usize> {
        self.type_index_dynamic(TypeId::of::<T>())
    }

    #[inline]
    fn type_index_dynamic(&self, type_id: TypeId) -> Option<usize> {
        self.type_indices.get(&type_id).cloned()
    }

    #[allow(missing_docs)]
    #[inline]
    pub fn get_storage<T: Component>(&self) -> Option<&dyn ComponentStorage> {
        self.type_index::<T>()
            .map(|index| &*self.component_storages[index])
    }

    // TODO: rename for parity
    #[allow(missing_docs)]
    #[inline]
    pub fn get_storage_dynamic(&self, type_id: TypeId) -> Option<&Box<dyn ComponentStorage>> {
        self.type_index_dynamic(type_id)
            .map(|index| &self.component_storages[index])
    }

    #[allow(missing_docs)]
    #[inline]
    pub fn get_storage_dynamic_mut(
        &mut self,
        type_id: TypeId,
    ) -> Option<&mut Box<dyn ComponentStorage>> {
        self.type_index_dynamic(type_id)
            .map(move |index| &mut self.component_storages[index])
    }

    #[allow(missing_docs)]
    #[inline]
    pub fn len(&self) -> usize {
        self.entities.len()
    }

    #[allow(missing_docs)]
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.entities.is_empty()
    }

    #[allow(missing_docs)]
    pub fn iter_entities(&self) -> impl Iterator<Item = &Entity> {
        self.entities.iter()
    }

    #[inline]
    pub(crate) fn get_entity(&self, index: usize) -> Entity {
        self.entities[index]
    }

    #[allow(missing_docs)]
    pub fn allocate(&mut self, entity: Entity) -> usize {
        if self.len() == self.entities.capacity() {
            self.reserve(self.grow_size);
        }

        self.entities.push(entity);
        for storage in self.component_storages.iter_mut() {
            storage.meta_mut().allocate();
        }

        self.len() - 1
    }

    #[inline]
    fn capacity(&self) -> usize {
        self.entities.len()
    }

    #[allow(missing_docs)]
    pub fn clear_trackers(&mut self) {
        for storage in self.component_storages.iter_mut() {
            storage.meta_mut().clear_trackers();
        }
    }

    pub fn reserve(&mut self, count: usize) {
        self.entities.reserve(count);

        for storage in self.component_storages.iter_mut() {
            storage.reserve(count);
            storage.meta_mut().reserve(count);
        }
    }

    /// Returns the ID of the entity moved into `index`, if any
    #[inline]
    pub(crate) fn remove(&mut self, index: usize) -> Option<Entity> {
        if index >= self.len() {
            panic!("entity index in archetype is out of bounds");
        }
        for storage in self.component_storages.iter_mut() {
            storage.swap_remove(index, false);
            let storage_meta = storage.meta_mut();
            storage_meta.added_entities.swap_remove(index);
            storage_meta.mutated_entities.swap_remove(index);
        }

        if self.entities.len() - 1 == index {
            self.entities.pop();
            None
        } else {
            self.entities.swap_remove(index);
            Some(self.entities[index])
        }
    }

    #[allow(missing_docs)]
    #[inline]
    pub unsafe fn move_to<'a, 'b>(
        &'a mut self,
        location: &'b mut Location,
        archetype: &'b mut Archetype,
        drop_unused: bool,
    ) -> Option<Entity> {
        if location.index >= self.len() {
            panic!("entity index in archetype is out of bounds");
        }

        let target_index = archetype.allocate(self.entities[location.index]);
        let old_index = std::mem::replace(&mut location.index, target_index);
        for storage in self.component_storages.iter_mut() {
            if let Some(target_storage) = archetype.get_storage_dynamic_mut(storage.get_type()) {
                let value = storage.get_value(old_index);
                target_storage.push(value);
                // forget the removed component because we copied it to the other archetype storage
                storage.swap_remove(old_index, true);
            } else {
                storage.swap_remove(old_index, !drop_unused);
            }
        }

        if self.entities.len() - 1 == old_index {
            self.entities.pop();
            None
        } else {
            self.entities.swap_remove(old_index);
            Some(self.entities[old_index])
        }
    }

    #[inline]
    pub unsafe fn get_value<T: Component>(&self, index: usize) -> Option<T> {
        self.get_storage::<T>()
            .map(|storage| storage.get_value(index).cast::<T>().read())
    }

    #[inline]
    pub fn insert<T: Component>(&mut self, value: T) {
        self.insert_dynamic(TypeId::of::<T>(), &value as *const T as *const u8);
        std::mem::forget(value);
    }

    #[inline]
    fn insert_dynamic(&mut self, type_id: TypeId, value: *const u8) {
        self.get_storage_dynamic_mut(type_id).unwrap().push(value);
    }

    /// How, if at all, `Q` will access entities in this archetype
    #[inline]
    pub fn access<Q: Query>(&self) -> Option<Access> {
        Q::Fetch::access(self)
    }
}

#[allow(missing_docs)]
#[derive(Debug)]
pub struct ComponentStorageMeta {
    pub borrow: AtomicBorrow,
    pub mutated_entities: Vec<bool>,
    pub added_entities: Vec<bool>,
}

impl Default for ComponentStorageMeta {
    fn default() -> Self {
        Self {
            borrow: AtomicBorrow::new(),
            mutated_entities: Vec::new(),
            added_entities: Vec::new(),
        }
    }
}

impl ComponentStorageMeta {
    #[allow(missing_docs)]
    pub fn clear_trackers(&mut self) {
        for mutated in self.mutated_entities.iter_mut() {
            *mutated = false;
        }

        for added in self.added_entities.iter_mut() {
            *added = false;
        }
    }

    fn reserve(&mut self, count: usize) {
        self.added_entities.reserve(count);
        self.mutated_entities.reserve(count);
    }

    pub fn allocate(&mut self) {
        self.added_entities.push(false);
        self.mutated_entities.push(false);
    }

    pub fn borrow(&self) {
        self.borrow.borrow();
    }

    pub fn borrow_mut(&self) {
        self.borrow.borrow_mut();
    }

    pub fn release(&self) {
        self.borrow.release();
    }

    pub fn release_mut(&self) {
        self.borrow.release_mut();
    }
}

#[derive(Clone, Debug)]
pub struct TypeInfo {
    id: TypeId,
    get_storage: fn() -> Box<dyn ComponentStorage>,
}

impl PartialEq for TypeInfo {
    fn eq(&self, other: &Self) -> bool {
        self.id.eq(&other.id)
    }
}

impl TypeInfo {
    pub fn of<T: Component>() -> Self {
        TypeInfo {
            id: TypeId::of::<T>(),
            get_storage: || Box::new(VecComponentStorage::<T>::default()),
        }
    }

    #[inline]
    pub fn id(&self) -> TypeId {
        self.id
    }
}
