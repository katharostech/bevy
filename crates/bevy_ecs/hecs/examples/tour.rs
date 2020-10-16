#![allow(unused_must_use)]

use bevy_hecs as h;

#[derive(Debug)]
struct Name(String);
#[derive(Debug)]
struct PlayerId(u32);
#[derive(Debug)]
struct Life(u32);

fn main() {
    // Welcome to bevy hecs here's a tour

    // first lets create a world to put stuff in
    let mut world = h::World::new();

    // now we can spawn entities
    println!("Spawning player1 and player2");
    let player1 = world.spawn((PlayerId(0), Name(String::from("John")), Life(100)));
    let player2 = world.spawn((PlayerId(1), Name(String::from("Jane")), Life(89)));

    dbg!(player1, player2);

    // We can query the world to access entities
    println!("Getting all players and printing their info");
    for (ent, pid, name, mut life) in
        &mut world.query_mut::<(h::Entity, &PlayerId, &Name, &mut Life)>()
    {
        life.0 += 1;

        println!(
            "entity: {:?}, playerId: {:?}, name: {:?}, life: {:?}",
            ent, pid, name, life
        );
    }

    // We can also get the values for components given their entity id
    println!("Getting player1 name");
    let name = world.get::<Name>(player1);

    // This will be a result because the entity may or may not have the component
    // we asked for
    dbg!(name);
}
