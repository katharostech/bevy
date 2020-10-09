use std::time::Duration;

use bytemuck::{Pod, Zeroable};

use bevy::prelude::*;
use bevy_app::ScheduleRunnerPlugin;
use bevy_ecs::{
    Access, ComponentId, DynamicComponentAccess, DynamicComponentInfo, DynamicComponentQuery,
};

// Define our componens

#[derive(Debug, Clone, Copy)]
struct Pos {
    x: f32,
    y: f32,
}

#[derive(Debug, Clone, Copy)]
struct Vel {
    x: f32,
    y: f32,
}

// Implement `Pod` ( Plain 'ol Data ) and `Zeroable` for our components so that we can cast them
// safely from raw bytes later

unsafe impl Zeroable for Pos {}
unsafe impl Zeroable for Vel {}
unsafe impl Pod for Pos {}
unsafe impl Pod for Vel {}

/// Create a system for spawning the scene
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

/// Create a system for printing the status of our entities
fn info(mut query: Query<(&Pos, &Vel)>) {
    println!("---");
    for (pos, vel) in &mut query.iter() {
        println!("{:?}\t\t{:?}", pos, vel);
    }
}

fn main() {
    // A Dynamic component query which can be constructed at runtime to represent which components
    // we want a dynamic system to access.
    //
    // Notice that the sizes and IDs of the components can be specified at runtime and allow for
    // storage of any data as an array of bytes.
    let mut query = DynamicComponentQuery::default();

    // Set the first element of the query to be write access to our `Pos` component
    query[0] = Some(DynamicComponentAccess {
        info: DynamicComponentInfo {
            id: ComponentId::RustTypeId(std::any::TypeId::of::<Pos>()),
            size: std::mem::size_of::<Pos>(),
        },
        access: Access::Write,
    });

    // Set the second element of the query to be read access to our `Vel` component
    query[1] = Some(DynamicComponentAccess {
        info: DynamicComponentInfo {
            id: ComponentId::RustTypeId(std::any::TypeId::of::<Vel>()),
            size: std::mem::size_of::<Vel>(),
        },
        access: Access::Read,
    });

    // Create our dynamic system by specifying the name, the query we created above, and a closure
    // that operates on the query
    let pos_vel_system = DynamicSystem::new("pos_vel_system".into(), query, |query| {
        // Iterate over the query just like you would in a typical query
        for mut components in &mut query.iter() {
            // `components` will be an array with indexes corresponding to the indexes of our
            // DynamicComponentAccess information that we constructed for our query when creating
            // the system.
            //
            // Each item in the array is an optional mutable reference to a byte slice representing
            // the component data: Option<&mut [u8]>.

            // Here we take the mutable reference to the bytes of our position and velocity
            // components
            let pos_bytes = components[0].take().unwrap();
            let vel_bytes = components[1].take().unwrap();

            // Instead of interacting with the raw bytes of our components, we first cast them to
            // their Rust structs
            let mut pos: &mut Pos = &mut bytemuck::cast_slice_mut(pos_bytes)[0];
            let vel: &Vel = &bytemuck::cast_slice(vel_bytes)[0];

            // Now we can operate on our components
            pos.x += vel.x;
            pos.y += vel.y;
        }
    });

    App::build()
        .add_plugin(ScheduleRunnerPlugin::run_loop(Duration::from_secs(1)))
        .add_startup_system(spawn_scene.thread_local_system())
        .add_system(Box::new(pos_vel_system))
        .add_system(info.system())
        .run();
}
