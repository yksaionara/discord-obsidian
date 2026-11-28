//! The rail's order as moved on this device: servers and folders keep the
//! place they were moved to, over Discord's order, and those not moved yet,
//! such as a server just joined, come first as Discord puts them. The order
//! is server and folder ids, each folder's servers after it.

use std::sync::Arc;

use crate::chat::{ChatServer, ServerFolder, ServerListEntry};

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
        if up {
            at.checked_sub(1)
        } else {
            (at + 1 < len).then_some(at + 1)
        }
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

/// Where a server or folder dragged on the rail lands, against the one
/// it's dropped on: above it, below it, or, for a server, into it, as
/// Discord's server list makes a folder of two servers dropped together.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Drop {
    Before,
    After,
    Into,
}

/// The rail as places a drop can change: each folder with its servers, and
/// each server outside one.
fn entries(servers: &[Arc<ChatServer>]) -> Vec<ServerListEntry> {
    places(servers)
        .into_iter()
        .map(|place| ServerListEntry {
            folder: place
                .folder
                .then(|| place.servers[0].folder.clone())
                .flatten(),
            servers: place
                .servers
                .iter()
                .map(|server| server.id.clone())
                .collect(),
        })
        .collect()
}

fn key(entry: &ServerListEntry) -> &str {
    entry
        .folder
        .as_ref()
        .map_or_else(|| entry.servers[0].as_str(), |folder| folder.id.as_str())
}

/// The rail with `id`, a server or folder, dropped on `target` as `drop`
/// says; none when nothing would change or it can't go there. A server
/// dropped into one outside any folder makes a folder of the two, with
/// `new_folder` its id; dropped into a folder, or into a server in one, it
/// joins that folder. A folder goes only beside others, as folders don't
/// hold folders. A folder its last server leaves goes.
pub fn dropped(
    servers: &[Arc<ChatServer>],
    id: &str,
    target: &str,
    drop: Drop,
    new_folder: &str,
) -> Option<Vec<ServerListEntry>> {
    if id == target {
        return None;
    }
    let before = entries(servers);
    let mut entries = before.clone();
    // Take what's dragged out.
    let dragged = match entries
        .iter()
        .position(|entry| entry.folder.is_some() && key(entry) == id)
    {
        Some(at) => entries.remove(at),
        None => {
            let at = entries
                .iter()
                .position(|entry| entry.servers.iter().any(|server| server == id))?;
            entries[at].servers.retain(|server| server != id);
            if entries[at].servers.is_empty() {
                entries.remove(at);
            }
            ServerListEntry {
                folder: None,
                servers: vec![id.to_owned()],
            }
        }
    };
    let folder_dragged = dragged.folder.is_some();
    if let Some(at) = entries.iter().position(|entry| key(entry) == target) {
        match (drop, folder_dragged, entries[at].folder.is_some()) {
            (Drop::Before, ..) => entries.insert(at, dragged),
            (Drop::After, ..) => entries.insert(at + 1, dragged),
            (Drop::Into, true, _) => return None,
            (Drop::Into, false, true) => entries[at].servers.push(id.to_owned()),
            (Drop::Into, false, false) => {
                let alone = entries[at].servers[0].clone();
                entries[at] = ServerListEntry {
                    folder: Some(ServerFolder {
                        id: new_folder.to_owned(),
                        name: None,
                        color: None,
                    }),
                    servers: vec![alone, id.to_owned()],
                };
            }
        }
    } else {
        // A server in a folder.
        let at = entries.iter().position(|entry| {
            entry.folder.is_some() && entry.servers.iter().any(|server| server == target)
        })?;
        if folder_dragged {
            match drop {
                Drop::Before => entries.insert(at, dragged),
                Drop::After => entries.insert(at + 1, dragged),
                Drop::Into => return None,
            }
        } else {
            let inside = &mut entries[at].servers;
            let place = inside.iter().position(|server| server == target)?;
            let place = if drop == Drop::Before {
                place
            } else {
                place + 1
            };
            inside.insert(place, id.to_owned());
        }
    }
    (entries != before).then_some(entries)
}

/// The rail with `id` one place up or down among its neighbours, as
/// [`dropped`] gives it: on the rail, or in its folder. None at an end.
pub fn moved_entries(
    servers: &[Arc<ChatServer>],
    id: &str,
    up: bool,
) -> Option<Vec<ServerListEntry>> {
    let order = moved(servers, id, up)?;
    let rank = |key: &str| order.iter().position(|kept| kept == key);
    let mut entries = entries(servers);
    entries.sort_by_key(|entry| rank(key(entry)));
    for entry in entries.iter_mut().filter(|entry| entry.folder.is_some()) {
        entry.servers.sort_by_key(|server| rank(server));
    }
    Some(entries)
}

/// The order [`arrange`] keeps for `entries`, for the rail on this device:
/// each place's key, a folder's servers after it.
pub fn order_of(entries: &[ServerListEntry]) -> Vec<String> {
    entries
        .iter()
        .flat_map(|entry| {
            std::iter::once(key(entry).to_owned()).chain(
                entry
                    .servers
                    .iter()
                    .filter(|_| entry.folder.is_some())
                    .cloned(),
            )
        })
        .collect()
}

/// An id for a new Discord folder, none of `taken`: a random number, as
/// Discord's client gives its folders, below 2^31.
pub fn new_folder_id(taken: &[&str]) -> String {
    let mut seed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_nanos() as u64);
    loop {
        // splitmix64: spreads the clock's bits over the number.
        seed = seed.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut mixed = seed;
        mixed = (mixed ^ (mixed >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        mixed = (mixed ^ (mixed >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        mixed ^= mixed >> 31;
        let id = format!("discord-folder:{}", (mixed % (1 << 31)).max(1));
        if !taken.contains(&id.as_str()) {
            return id;
        }
    }
}

/// Whether `id` is a folder on the rail.
pub fn is_folder(servers: &[Arc<ChatServer>], id: &str) -> bool {
    servers
        .iter()
        .any(|server| server.folder.as_ref().is_some_and(|folder| folder.id == id))
}

/// Whether `id` can move up, and down.
pub fn can_move(servers: &[Arc<ChatServer>], id: &str) -> (bool, bool) {
    (
        moved(servers, id, true).is_some(),
        moved(servers, id, false).is_some(),
    )
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

    /// The rail as `entries` say: a folder as its id and its servers in
    /// brackets, a server by its id.
    fn shape(entries: &[ServerListEntry]) -> Vec<String> {
        entries
            .iter()
            .map(|entry| match &entry.folder {
                Some(folder) => format!("{}[{}]", folder.id, entry.servers.join(",")),
                None => entry.servers.join(","),
            })
            .collect()
    }

    #[test]
    fn servers_drop_beside_and_into_each_other_as_on_discord() {
        let rail = vec![
            server("a", None),
            server("b", Some("f")),
            server("c", Some("f")),
            server("d", None),
        ];
        let drop = |id, target, drop| {
            dropped(&rail, id, target, drop, "new").map(|entries| shape(&entries))
        };
        assert_eq!(drop("d", "a", Drop::Before).unwrap(), ["d", "a", "f[b,c]"]);
        assert_eq!(drop("a", "d", Drop::After).unwrap(), ["f[b,c]", "d", "a"]);
        assert_eq!(
            drop("a", "d", Drop::Into).unwrap(),
            ["f[b,c]", "new[d,a]"],
            "two servers make a folder"
        );
        assert_eq!(
            drop("d", "f", Drop::Into).unwrap(),
            ["a", "f[b,c,d]"],
            "into a folder, last"
        );
        assert_eq!(
            drop("d", "b", Drop::Before).unwrap(),
            ["a", "f[d,b,c]"],
            "into a folder, between"
        );
        assert_eq!(
            drop("c", "a", Drop::Before).unwrap(),
            ["c", "a", "f[b]", "d"],
            "out of a folder"
        );
        assert_eq!(
            drop("f", "a", Drop::Before).unwrap(),
            ["f[b,c]", "a", "d"],
            "a folder, whole"
        );
        assert_eq!(drop("f", "d", Drop::Into), None, "folders hold no folders");
        assert_eq!(drop("a", "a", Drop::Before), None);
        assert_eq!(drop("a", "f", Drop::Before), None, "already there");
        // The folder's last server leaves: the folder goes.
        let rail = vec![server("a", None), server("b", Some("f"))];
        assert_eq!(
            shape(&dropped(&rail, "b", "a", Drop::After, "new").unwrap()),
            ["a", "b"]
        );
        let entries = dropped(&rail, "a", "b", Drop::Before, "new").unwrap();
        assert_eq!(shape(&entries), ["f[a,b]"]);
        assert_eq!(order_of(&entries), ["f", "a", "b"]);
        assert_eq!(
            shape(&moved_entries(&rail, "f", true).unwrap()),
            ["f[b]", "a"]
        );
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
