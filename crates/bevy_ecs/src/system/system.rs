use crate::{resource::Resources, GenericQuery};
use bevy_hecs::{Access, ComponentId, DynamicComponentQuery, Fetch, Query, World};
use bevy_utils::HashSet;
use fixedbitset::FixedBitSet;
use std::borrow::Cow;

/// Determines the strategy used to run the `run_thread_local` function in a [System]
#[derive(Copy, Clone, Eq, PartialEq, Debug)]
pub enum ThreadLocalExecution {
    Immediate,
    NextFlush,
}

#[derive(Debug, Copy, Clone, Eq, PartialEq, Hash)]
pub struct SystemId(pub usize);

impl SystemId {
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        SystemId(rand::random::<usize>())
    }
}

/// An ECS system that can be added to a [Schedule](crate::Schedule)
pub trait System: Send + Sync {
    fn name(&self) -> Cow<'static, str>;
    fn id(&self) -> SystemId;
    fn update_archetype_access(&mut self, world: &World);
    fn archetype_access(&self) -> &ArchetypeAccess;
    fn resource_access(&self) -> &TypeAccess;
    fn thread_local_execution(&self) -> ThreadLocalExecution;
    fn run(&mut self, world: &World, resources: &Resources);
    fn run_thread_local(&mut self, world: &mut World, resources: &mut Resources);
    fn initialize(&mut self, _world: &mut World, _resources: &mut Resources) {}
}

/// Provides information about the archetypes a [System] reads and writes
#[derive(Debug, Default)]
pub struct ArchetypeAccess {
    pub accessed: FixedBitSet, // union of both immutable and mutable
    pub mutable: FixedBitSet,
}

// credit to Ratysz from the Yaks codebase
impl ArchetypeAccess {
    pub fn is_compatible(&self, other: &ArchetypeAccess) -> bool {
        self.mutable.is_disjoint(&other.accessed) && self.accessed.is_disjoint(&other.mutable)
    }

    pub fn union(&mut self, other: &ArchetypeAccess) {
        self.mutable.union_with(&other.mutable);
        self.accessed.union_with(&other.accessed);
    }

    pub fn set_access_for_query<Q>(&mut self, world: &World)
    where
        Q: Query,
        Q::Fetch: for<'a> Fetch<'a, State = ()>,
    {
        self.set_access_for_stateful_query::<(), Q>(world, &());
    }

    pub fn set_access_for_stateful_query<S, Q>(&mut self, world: &World, state: &S)
    where
        S: Default,
        Q: Query,
        Q::Fetch: for<'a> Fetch<'a, State = S>,
    {
        let iterator = world.archetypes();
        let bits = iterator.len();
        self.accessed.grow(bits);
        self.mutable.grow(bits);
        iterator
            .enumerate()
            .filter_map(|(index, archetype)| {
                archetype
                    .access::<S, Q>(state)
                    .map(|access| (index, access))
            })
            .for_each(|(archetype, access)| match access {
                Access::Read => self.accessed.set(archetype, true),
                Access::Write => {
                    self.accessed.set(archetype, true);
                    self.mutable.set(archetype, true);
                }
                Access::Iterate => (),
            });
    }

    pub fn clear(&mut self) {
        self.accessed.clear();
        self.mutable.clear();
    }
}

/// Provides information about the types a [System] reads and writes
#[derive(Debug, Default, Eq, PartialEq, Clone)]
pub struct TypeAccess {
    pub immutable: HashSet<ComponentId>,
    pub mutable: HashSet<ComponentId>,
}

impl TypeAccess {
    pub fn is_compatible(&self, other: &TypeAccess) -> bool {
        self.mutable.is_disjoint(&other.mutable)
            && self.mutable.is_disjoint(&other.immutable)
            && self.immutable.is_disjoint(&other.mutable)
    }

    pub fn union(&mut self, other: &TypeAccess) {
        self.mutable.extend(&other.mutable);
        self.immutable.extend(&other.immutable);
    }

    pub fn clear(&mut self) {
        self.immutable.clear();
        self.mutable.clear();
    }
}

pub struct DynamicSystem<S> {
    pub name: String,
    pub state: S,
    system_id: SystemId,
    archetype_access: ArchetypeAccess,
    resource_access: TypeAccess,
    settings: DynamicSystemSettings<S>,
}

#[derive(Clone)]
pub struct DynamicSystemSettings<S> {
    pub workload:
        fn(&mut S, &Resources, &mut [GenericQuery<DynamicComponentQuery, DynamicComponentQuery>]),
    pub queries: Vec<DynamicComponentQuery>,
    pub thread_local_execution: ThreadLocalExecution,
    pub thread_local_system: fn(&mut S, &mut World, &mut Resources),
    pub init_function: fn(&mut S, &mut World, &mut Resources),
    pub resource_access: TypeAccess,
}

impl<S> Default for DynamicSystemSettings<S> {
    fn default() -> Self {
        Self {
            workload: |_, _, _| (),
            queries: Default::default(),
            thread_local_execution: ThreadLocalExecution::NextFlush,
            thread_local_system: |_, _, _| (),
            init_function: |_, _, _| (),
            resource_access: Default::default(),
        }
    }
}

impl<S> DynamicSystem<S> {
    pub fn new(name: String, state: S) -> Self {
        DynamicSystem {
            name,
            state,
            system_id: SystemId::new(),
            resource_access: Default::default(),
            archetype_access: Default::default(),
            settings: Default::default(),
        }
    }

    pub fn settings(mut self, settings: DynamicSystemSettings<S>) -> Self {
        self.settings = settings;
        self
    }
}

impl<S: Send + Sync> System for DynamicSystem<S> {
    fn name(&self) -> std::borrow::Cow<'static, str> {
        self.name.clone().into()
    }

    fn id(&self) -> SystemId {
        self.system_id
    }

    fn update_archetype_access(&mut self, world: &World) {
        // Clear previous archetype access list
        self.archetype_access.clear();

        for query in &self.settings.queries {
            self.archetype_access
                .set_access_for_stateful_query::<_, DynamicComponentQuery>(&world, &query);
        }
    }

    fn archetype_access(&self) -> &ArchetypeAccess {
        &self.archetype_access
    }

    fn resource_access(&self) -> &TypeAccess {
        &self.resource_access
    }

    fn thread_local_execution(&self) -> ThreadLocalExecution {
        self.settings.thread_local_execution
    }

    fn run(&mut self, world: &World, resources: &Resources) {
        let archetype_access = &self.archetype_access;
        let mut queries: Vec<_> = self
            .settings
            .queries
            .iter()
            .map(|query| GenericQuery::new_stateful(world, &archetype_access, query))
            .collect();

        (self.settings.workload)(&mut self.state, resources, queries.as_mut_slice());
    }

    fn run_thread_local(&mut self, world: &mut World, resources: &mut Resources) {
        (self.settings.thread_local_system)(&mut self.state, world, resources);
    }

    fn initialize(&mut self, world: &mut World, resources: &mut Resources) {
        (self.settings.init_function)(&mut self.state, world, resources);
    }
}

#[cfg(test)]
mod tests {
    use super::{ArchetypeAccess, TypeAccess};
    use crate::resource::{FetchResource, Res, ResMut, ResourceQuery};
    use bevy_hecs::World;
    use std::any::TypeId;

    struct A;
    struct B;
    struct C;

    #[test]
    fn query_archetype_access() {
        let mut world = World::default();
        let e1 = world.spawn((A,));
        let e2 = world.spawn((A, B));
        let e3 = world.spawn((A, B, C));

        let mut access = ArchetypeAccess::default();
        access.set_access_for_query::<(&A,)>(&world);

        let e1_archetype = world.get_entity_location(e1).unwrap().archetype as usize;
        let e2_archetype = world.get_entity_location(e2).unwrap().archetype as usize;
        let e3_archetype = world.get_entity_location(e3).unwrap().archetype as usize;

        assert!(access.accessed.contains(e1_archetype));
        assert!(access.accessed.contains(e2_archetype));
        assert!(access.accessed.contains(e3_archetype));

        let mut access = ArchetypeAccess::default();
        access.set_access_for_query::<(&A, &B)>(&world);

        assert!(access.accessed.contains(e1_archetype) == false);
        assert!(access.accessed.contains(e2_archetype));
        assert!(access.accessed.contains(e3_archetype));
    }

    #[test]
    fn resource_query_access() {
        let access =
            <<(Res<A>, ResMut<B>, Res<C>) as ResourceQuery>::Fetch as FetchResource>::access();
        let mut expected_access = TypeAccess::default();
        expected_access.immutable.insert(TypeId::of::<A>().into());
        expected_access.immutable.insert(TypeId::of::<C>().into());
        expected_access.mutable.insert(TypeId::of::<B>().into());
        assert_eq!(access, expected_access);
    }
}
