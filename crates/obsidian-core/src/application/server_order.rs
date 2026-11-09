//! The rail's order as moved on this device: servers and folders keep the
//! place they were moved to, over Discord's order, and those not moved yet,
//! such as a server just joined, come first as Discord puts them. The order
//! is server and folder ids, each folder's servers after it.

use std::sync::Arc;

use crate::chat::ChatServer;

/// One place on the rail: a server alone, or a folder's servers.
struct Place {
    /// The server's id, or the folder's.
    key: String,
    servers: Vec<Arc<ChatServer>>,
    folder: bool,
}

/// `servers` as the rail shows them, in places.
fn places(servers: &[Arc<ChatServer>]) -> Vec<Place> {
    let mut places: Vec<Place> = Vec::new();
    for server in servers {
        match &server.folder {
            Some(folder) => match places.last_mut() {
                Some(place) if place.folder && place.key == folder.id => {
                    place.servers.push(server.clone());
                }
                _ => places.push(Place {
                    key: folder.id.clone(),
                    servers: vec![server.clone()],
                    folder: true,
                }),
            },
            None => places.push(Place {
                key: server.id.clone(),
                servers: vec![server.clone()],
                folder: false,
            }),
        }
    }
    places
}

fn flatten(places: Vec<Place>) -> Vec<Arc<ChatServer>> {
    places.into_iter().flat_map(|place| place.servers).collect()
}

/// Where `key` sits in `order`: those not in it first, as they came.
fn rank(order: &[String], key: &str) -> Option<usize> {
    order.iter().position(|kept| kept == key)
}

/// Puts `servers` in the order kept: each place where it was moved, and
/// each folder's servers likewise.
pub fn arrange(servers: &mut Vec<Arc<ChatServer>>, order: &[String]) {
    if order.is_empty() {
        return;
    }
    let mut places = places(servers);
    // Stable: those not moved keep Discord's order among themselves.
    places.sort_by_key(|place| rank(order, &place.key));
    for place in places.iter_mut().filter(|place| place.folder) {
        place.servers.sort_by_key(|server| rank(order, &server.id));
    }
    *servers = flatten(places);
}

/// The order with `id`, a server or folder, one place up or down among its
/// neighbours: on the rail, or in its folder. None at an end.
pub fn moved(servers: &[Arc<ChatServer>], id: &str, up: bool) -> Option<Vec<String>> {
    let mut places = places(servers);
    let step = |at: usize, len: usize| -> Option<usize> {
        if up { at.checked_sub(1) } else { (at + 1 < len).then_some(at + 1) }
    };
    if let Some(at) = places.iter().position(|place| place.key == id) {
        let to = step(at, places.len())?;
        places.swap(at, to);
    } else {
        let place = places
            .iter_mut()
            .find(|place| place.folder && place.servers.iter().any(|server| server.id == id))?;
        let at = place.servers.iter().position(|server| server.id == id)?;
        let to = step(at, place.servers.len())?;
        place.servers.swap(at, to);
    }
    Some(
        places
            .iter()
            .flat_map(|place| {
                std::iter::once(place.key.clone()).chain(
                    place
                        .servers
                        .iter()
                        .filter(|_| place.folder)
                        .map(|server| server.id.clone()),
                )
            })
            .collect(),
    )
}

/// Whether `id` is a folder on the rail.
pub fn is_folder(servers: &[Arc<ChatServer>], id: &str) -> bool {
    servers
        .iter()
        .any(|server| server.folder.as_ref().is_some_and(|folder| folder.id == id))
}

/// Whether `id` can move up, and down.
pub fn can_move(servers: &[Arc<ChatServer>], id: &str) -> (bool, bool) {
    (moved(servers, id, true).is_some(), moved(servers, id, false).is_some())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat::ServerFolder;

    fn server(id: &str, folder: Option<&str>) -> Arc<ChatServer> {
        Arc::new(ChatServer {
            id: id.into(),
            folder: folder.map(|folder| ServerFolder {
                id: folder.into(),
                name: None,
                color: None,
            }),
            ..Default::default()
        })
    }

    fn ids(servers: &[Arc<ChatServer>]) -> Vec<&str> {
        servers.iter().map(|server| server.id.as_str()).collect()
    }

    #[test]
    fn servers_and_folders_move_and_keep_their_places() {
        let discord = vec![
            server("a", None),
            server("b", Some("f")),
            server("c", Some("f")),
            server("d", None),
        ];
        // The folder moves up above a, its servers with it.
        let order = moved(&discord, "f", true).unwrap();
        assert_eq!(order, ["f", "b", "c", "a", "d"]);
        let mut servers = discord.clone();
        arrange(&mut servers, &order);
        assert_eq!(ids(&servers), ["b", "c", "a", "d"]);
        // A server in a folder moves within it, and no further.
        let order = moved(&servers, "c", true).unwrap();
        arrange(&mut servers, &order);
        assert_eq!(ids(&servers), ["c", "b", "a", "d"]);
        assert_eq!(moved(&servers, "c", true), None, "at the folder's top");
        assert_eq!(moved(&servers, "d", false), None, "at the bottom");
        assert_eq!(can_move(&servers, "a"), (true, true));
        assert!(is_folder(&servers, "f") && !is_folder(&servers, "a"));

        // Discord's next list, with a server just joined: it comes first,
        // and the rest keep where they were moved.
        let mut next = vec![server("new", None)];
        next.extend(discord);
        arrange(&mut next, &order);
        assert_eq!(ids(&next), ["new", "c", "b", "a", "d"]);
    }
}
