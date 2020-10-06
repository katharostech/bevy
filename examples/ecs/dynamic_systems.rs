use std::{
    any::{Any, TypeId},
    time::Duration,
};

use bevy::prelude::*;
use bevy_app::ScheduleRunnerPlugin;
use bevy_ecs::{
    Access, ArchetypeAccess, ComponentId, Fetch, HecsQuery, SystemId, ThreadLocalExecution,
    TypeAccess,
};

#[derive(Debug)]
struct Pos {
    x: f32,
    y: f32,
}

#[derive(Debug)]
struct Vel {
    x: f32,
    y: f32,
}

type SystemWorkload = fn(&mut DynamicSystem, &World, &Resources);

struct ComponentAccess {
    id: ComponentId,
    access: Access,
}

struct DynamicSystem {
    name: String,
    system_id: SystemId,
    workload: SystemWorkload,
    archetype_access: ArchetypeAccess,
    resource_access: TypeAccess,
    component_accesses: Vec<ComponentAccess>,
}

impl DynamicSystem {
    fn new(
        name: String,
        component_accesses: Vec<ComponentAccess>,
        workload: SystemWorkload,
    ) -> Self {
        DynamicSystem {
            name,
            workload,
            component_accesses,
            resource_access: Default::default(),
            archetype_access: Default::default(),
            system_id: SystemId::new(),
        }
    }
}

impl System for DynamicSystem {
    /// Get the system name
    fn name(&self) -> std::borrow::Cow<'static, str> {
        self.name.clone().into()
    }

    /// Get the system ID
    fn id(&self) -> SystemId {
        self.system_id
    }

    /// Have the system update it's record of what world archetypes it needs to
    /// access
    fn update_archetype_access(&mut self, world: &World) {
        // Clear previous archetype access list
        self.archetype_access.clear();

        // Iterate over the world's archetypes
        let iterator = world.archetypes();
        
        // Make sure we grow our bitsets to the size of the world archetype count
        let bits = iterator.len();
        self.archetype_access.immutable.grow(bits);
        self.archetype_access.mutable.grow(bits);

        // Go through all of the archetypes and check whether or not we are
        // reading or writing to them
        for (index, archetype) in iterator.enumerate() {
            for component_access in &self.component_accesses {
                if archetype.has_component(component_access.id) {
                    match component_access.access {
                        Access::Read => self.archetype_access.immutable.set(index, true),
                        Access::Write => self.archetype_access.mutable.set(index, true),
                        Access::Iterate => (),
                    }
                }
            }
        }
    }

    /// Get the archetypes that the system needs to access
    fn archetype_access(&self) -> &ArchetypeAccess {
        &self.archetype_access
    }

    /// Get the resources that the system needs to access
    fn resource_access(&self) -> &TypeAccess {
        // FIXME: Allow defining resource access
        &self.resource_access
    }

    /// Indicate when the thread local system is meant to run
    fn thread_local_execution(&self) -> ThreadLocalExecution {
        ThreadLocalExecution::NextFlush
    }

    /// Run the system
    fn run(&mut self, world: &World, resources: &Resources) {
        (self.workload)(self, world, resources);
    }

    /// Run the thread local system, if any
    fn run_thread_local(&mut self, world: &mut World, resources: &mut Resources) {}

    /// Initialize the system, if necessary
    fn initialize(&mut self, _world: &mut World, _resources: &mut Resources) {}
}

fn spawn_scene(world: &mut World, _resources: &mut Resources) {
    #[rustfmt::skip]
    world.spawn_batch(vec![
        (
            Pos {
                x: 0.0,
                y: 0.0
            },
            Vel {
                x: 0.0,
                y: 1.0,
            }
        ),
        (
            Pos {
                x: 0.0,
                y: 0.0
            },
            Vel {
                x: 0.0,
                y: -1.0,
            }
        )
    ]);
}

fn apply_velocities(mut query: Query<(&mut Pos, &Vel)>) {
    for (mut pos, vel) in &mut query.iter() {
        pos.x += vel.x;
        pos.y += vel.y;
    }
}

fn info(mut query: Query<(&Pos, &Vel)>) {
    println!("---");
    for (pos, vel) in &mut query.iter() {
        println!("{:?}\t\t{:?}", pos, vel);
    }
}

fn main() {
    App::build()
        .add_plugin(ScheduleRunnerPlugin::run_loop(Duration::from_secs(1)))
        .add_startup_system(spawn_scene.thread_local_system())
        .add_system(apply_velocities.system())
        .add_system(info.system())
        .run();
}
