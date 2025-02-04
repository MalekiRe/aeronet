// lib.rs or networking.rs
use bevy::app::App;
use bevy::prelude::*;
use bevy_matchbox::matchbox_socket::WebRtcSocket;
use bevy_matchbox::prelude::PeerId;
use bevy_matchbox::MatchboxSocket;
use futures::channel::mpsc::SendError;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// Core traits and types
//----------------------------------------

#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug)]
pub enum Reliability {
    Reliable,
    Unreliable,
}

pub trait NetworkedEvent {
    fn id(&self) -> PeerId;
    fn reliable(&self) -> Reliability;
}

#[derive(Serialize, Deserialize)]
pub struct Message {
    pub type_name: String,
    pub content: Vec<u8>,
}
impl Message {
    pub fn new<T: Serialize + TypePath>(content: &T) -> Self {
        Message {
            type_name: T::type_path().to_string(),
            content: bincode::serialize(content).unwrap(),
        }
    }
}

// Socket message handling trait and implementation
//----------------------------------------

pub trait SocketSendMessage {
    fn send_msg_unreliable<T: Serialize + TypePath + Event + NetworkedEvent>(
        &mut self,
        peer: PeerId,
        message: &T,
    );
    fn send_msg_reliable<T: Serialize + TypePath + Event + NetworkedEvent>(
        &mut self,
        peer: PeerId,
        message: &T,
    );
    fn receive_msg_reliable(&mut self) -> Vec<(PeerId, Message)>;
    fn receive_msg_unreliable(&mut self) -> Vec<(PeerId, Message)>;
    fn send_msg_all_reliable<T: Serialize + TypePath + Event + NetworkedEvent>(
        &mut self,
        message: &T,
    );
    fn send_msg_all_unreliable<T: Serialize + TypePath + Event + NetworkedEvent>(
        &mut self,
        message: &T,
    );
    fn try_send_msg_all_reliable<T: Serialize + TypePath + Event + NetworkedEvent>(
        &mut self,
        message: &T,
    ) -> Result<(), SendError>;
    fn try_send_msg_reliable<T: Serialize + TypePath + Event + NetworkedEvent>(
        &mut self,
        peer: PeerId,
        message: &T,
    ) -> Result<(), SendError>;
}

impl SocketSendMessage for WebRtcSocket {
    fn send_msg_unreliable<T: Serialize + TypePath + Event + NetworkedEvent>(
        &mut self,
        peer: PeerId,
        message: &T,
    ) {
        let msg = Message::new(message);
        let msg = bincode::serialize(&msg).unwrap();
        self.channel_mut(1).send(msg.into(), peer);
    }

    fn send_msg_reliable<T: Serialize + TypePath + Event + NetworkedEvent>(
        &mut self,
        peer: PeerId,
        message: &T,
    ) {
        let msg = Message::new(message);
        let msg = bincode::serialize(&msg).unwrap();
        self.channel_mut(0).send(msg.into(), peer);
    }

    fn receive_msg_reliable(&mut self) -> Vec<(PeerId, Message)> {
        self.channel_mut(0)
            .receive()
            .into_iter()
            .map(|(id, packet)| (id, bincode::deserialize(&packet).unwrap()))
            .collect()
    }

    fn receive_msg_unreliable(&mut self) -> Vec<(PeerId, Message)> {
        self.channel_mut(1)
            .receive()
            .into_iter()
            .map(|(id, packet)| (id, bincode::deserialize(&packet).unwrap()))
            .collect()
    }

    fn send_msg_all_reliable<T: Serialize + TypePath + Event + NetworkedEvent>(
        &mut self,
        message: &T,
    ) {
        let peers = self.connected_peers().collect::<Vec<_>>();
        for peer in peers {
            self.send_msg_reliable(peer, message);
        }
    }

    fn send_msg_all_unreliable<T: Serialize + TypePath + Event + NetworkedEvent>(
        &mut self,
        message: &T,
    ) {
        let peers = self.connected_peers().collect::<Vec<_>>();
        for peer in peers {
            self.send_msg_unreliable(peer, message);
        }
    }

    fn try_send_msg_all_reliable<T: Serialize + TypePath + Event + NetworkedEvent>(
        &mut self,
        message: &T,
    ) -> Result<(), SendError> {
        let peers = self.connected_peers().collect::<Vec<_>>();
        for peer in peers {
            self.try_send_msg_reliable(peer, message)?;
        }
        Ok(())
    }

    fn try_send_msg_reliable<T: Serialize + TypePath + Event + NetworkedEvent>(
        &mut self,
        peer: PeerId,
        message: &T,
    ) -> Result<(), SendError> {
        let msg = Message::new(message);
        let msg = bincode::serialize(&msg).unwrap();
        self.channel_mut(0).try_send(msg.into(), peer)?;
        Ok(())
    }
}

// Message routing and handling
//----------------------------------------

#[derive(Default, Resource)]
pub struct NetworkedMessages(
    HashMap<
        String,
        (
            Box<dyn Fn(&mut World, &[u8]) + Send + Sync + 'static>,
            Box<dyn Fn(&mut World) + Send + Sync + 'static>,
        ),
    >,
);

fn route_outgoing_messages<
    T: NetworkedEvent + Event + Serialize + for<'a> Deserialize<'a> + TypePath,
>(
    world: &mut World,
) {
    world.resource_scope(|world, mut socket: Mut<MatchboxSocket>| {
        let events = world.get_resource_mut::<Events<T>>().unwrap();
        let mut cursor = events.get_cursor();
        for e in cursor.read(&events) {
            if e.id() == socket.id().unwrap() {
                match e.reliable() {
                    Reliability::Reliable => socket.send_msg_all_reliable(e),
                    Reliability::Unreliable => socket.send_msg_all_unreliable(e),
                }
            }
        }
    });
}

fn route_incoming_messages<T: NetworkedEvent + Event + for<'a> Deserialize<'a>>(
    world: &mut World,
    message: &[u8],
) {
    let e: T = bincode::deserialize(message).unwrap();
    world.send_event(e);
}

fn route_messages(world: &mut World) {
    if !world.contains_resource::<MatchboxSocket>() {
        return;
    }

    world.resource_scope(|world, networked_messages: Mut<NetworkedMessages>| {
        world.resource_scope(|world, mut socket: Mut<MatchboxSocket>| {
            for (_peer_id, msg) in socket.receive_msg_reliable() {
                let func = networked_messages.0.get(&msg.type_name).unwrap();
                func.0(world, &msg.content);
            }
            for (_peer_id, msg) in socket.receive_msg_unreliable() {
                let func = networked_messages.0.get(&msg.type_name).unwrap();
                func.0(world, &msg.content);
            }
        });
        for (_, route_outgoing_messages) in networked_messages.0.values() {
            route_outgoing_messages(world);
        }
    });
    let _peers = world
        .get_resource_mut::<MatchboxSocket>()
        .unwrap()
        .update_peers();
}

#[derive(Clone, Copy, Resource, Default)]
pub struct LocalId(Option<PeerId>);
impl LocalId {
    pub fn get(&self) -> Option<PeerId> {
        self.0
    }
}
impl std::ops::Deref for LocalId {
    type Target = Option<PeerId>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

fn local_id_set(mut local_id: ResMut<LocalId>, matchbox_socket: Option<ResMut<MatchboxSocket>>) {
    if let Some(mut matchbox_socket) = matchbox_socket {
        local_id.0 = matchbox_socket.id();
    } else {
        local_id.0.take();
    }
}

// Plugin implementation and app extension
//----------------------------------------

pub struct EvnetPlugin;

impl Plugin for EvnetPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, (local_id_set, route_messages).chain());
        app.init_resource::<NetworkedMessages>();
        app.init_resource::<LocalId>();
    }
}

pub trait NetworkedAppExt {
    fn register_networked_event<
        T: NetworkedEvent + Event + for<'a> Deserialize<'a> + Serialize + TypePath,
    >(
        &mut self,
    ) -> &mut Self;
}

impl NetworkedAppExt for App {
    fn register_networked_event<
        T: NetworkedEvent + Event + for<'a> Deserialize<'a> + Serialize + TypePath,
    >(
        &mut self,
    ) -> &mut Self {
        self.init_resource::<NetworkedMessages>();
        let mut networked_messages = self
            .main_mut()
            .world_mut()
            .get_resource_mut::<NetworkedMessages>()
            .unwrap();
        networked_messages.0.insert(
            T::type_path().to_string(),
            (
                Box::new(route_incoming_messages::<T>),
                Box::new(route_outgoing_messages::<T>),
            ),
        );
        self
    }
}

pub trait NetworkedCommandExt {
    fn connect(&mut self, room: &str);
}

impl NetworkedCommandExt for Commands<'_, '_> {
    fn connect(&mut self, room_url: &str) {
        let matchbox = MatchboxSocket::from(
            //example: "wss://mb.v-sekai.cloud/my-room-1"
            bevy_matchbox::matchbox_socket::WebRtcSocketBuilder::new(room_url)
                .add_reliable_channel()
                .add_unreliable_channel()
                .build(),
        );
        self.insert_resource(matchbox);
    }
}
