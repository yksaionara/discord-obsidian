//! A Discord forum, which shows its posts in place of messages, as Discord's
//! client shows one: its active posts, pinned ones first, then its older,
//! archived ones a page at a time, or the posts a search of their titles
//! finds. Each post shows as a card with its first message. The active posts
//! are the server's, as its snapshot shares them; their first messages are
//! asked for ten at a time, for the cards near the top and then for more as
//! the list scrolls, as Discord's client asks for those it shows. Archived
//! pages and a search's results bring their own. All of it is kept only
//! while the forum shows.

use std::collections::HashMap;

use super::*;
use crate::chat::{ForumTag, ListedThread, PostStarter, PostStarters};
use threads::ThreadList;

/// How many cards want their first messages at first, and how many more
/// each time the list scrolls near its end: a page.
const CARDS: usize = 25;
/// How many posts' first messages one request asks for, as Discord's client
/// asks for them.
const STARTERS_PER_ASK: usize = 10;

/// The forum on screen.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ForumView {
    /// Its archived posts and a search, as a channel's list of threads keeps
    /// them.
    pub list: ThreadList,
    /// Posts' first messages, by room id; none where it was deleted.
    pub starters: HashMap<String, Option<PostStarter>>,
    /// Posts whose first messages are on their way.
    asking: Vec<String>,
    /// Asking failed: it waits for the list to scroll before asking again.
    stalled: bool,
    /// How many cards, from the top, want their first messages.
    shown: usize,
    /// The last message read here when it opened, a snowflake: posts made
    /// after it, which this account never opened, are new.
    read: u64,
}

/// How the forum on screen sorts, lays out and filters its posts, as Discord's
/// Sort & View and its tags choose: each `None` follows the forum's default.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ForumChoice {
    /// Date Posted rather than Recently Active.
    pub by_creation: Option<bool>,
    /// Gallery rather than List.
    pub gallery: Option<bool>,
    /// Match All rather than Match Some.
    pub match_all: Option<bool>,
    /// The tags chosen to filter by, by id.
    pub tags: Vec<String>,
}

/// A forum's posts as it shows them.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct ForumPosts {
    /// Its active posts, pinned ones first, then by their latest message,
    /// or by when they were made, as the forum sorts them.
    pub active: Vec<ForumPost>,
    /// Older, archived ones, by their latest message.
    pub older: Vec<ForumPost>,
    /// What a search of their titles found, while one shows: `None` while
    /// it's on its way, or why it failed.
    pub found: Option<Option<Result<Vec<ForumPost>, String>>>,
}

/// A post as its card shows it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct ForumPost {
    pub thread: ListedThread,
    /// Its tags, in the forum's order.
    pub tags: Vec<ForumTag>,
    /// Its first message: `None` until it comes, `Some(None)` when it was
    /// deleted.
    pub starter: Option<Option<PostStarter>>,
    /// Made since the forum was last read, and never opened.
    pub new: bool,
}

/// What a forum says of its posts, besides them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ForumInfo {
    pub tags: Vec<ForumTag>,
    pub by_creation: bool,
    /// A post must take one of its tags.
    pub require_tag: bool,
    /// The emoji a post without reactions offers.
    pub reaction: Option<chat::ShownEmoji>,
    /// Its posts show as a gallery by default; a media channel's only do.
    pub gallery: bool,
    pub media: bool,
    /// Several tags filter for posts with all of them by default.
    pub match_all: bool,
}

/// How the forum on screen shows now: its choices, or its defaults.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ForumShown {
    pub by_creation: bool,
    pub gallery: bool,
    pub match_all: bool,
    /// The tags filtering, by id.
    pub tags: Vec<String>,
}

impl ForumView {
    /// A page of posts, or first messages, is on its way.
    pub fn loading(&self) -> bool {
        self.list.loading || !self.asking.is_empty()
    }

    /// Scrolling further wants more: of `cards`, some beyond those that
    /// have asked for their first messages, or older posts still to load.
    pub fn wants_more(&self, cards: usize) -> bool {
        self.list.search.is_none() && (self.list.more || cards > self.shown)
    }
}

impl AppState {
    /// The Discord forum on screen.
    pub fn forum(&self) -> Option<&ForumView> {
        self.forum
            .as_ref()
            .filter(|forum| self.selected.as_ref() == Some(&forum.list.room))
    }
}

impl Application {
    /// Shows a forum, by room id, in place of messages: its posts, and the
    /// first messages of those on top. Discord sends its active posts, and
    /// any made while it shows, with the server's threads.
    pub(super) fn show_forum(&mut self, room: String) -> Vec<Effect> {
        let archived = self.may(&room, chat::Permission::ReadHistory);
        let (list, mut effects) = ThreadList::open(room.clone(), archived);
        self.state.forum = Some(ForumView {
            list,
            shown: CARDS,
            read: self.forum_read(&room),
            ..ForumView::default()
        });
        effects.push(Effect::Backend(Command::Discord(
            DiscordCommand::ViewForum { room },
        )));
        effects.extend(self.ask_starters());
        effects
    }

    /// Forgets the forum, as another room shows.
    pub(super) fn close_forum(&mut self) -> Vec<Effect> {
        match self.state.forum.take() {
            Some(_) => vec![Effect::Backend(Command::Discord(
                DiscordCommand::CloseThreads,
            ))],
            None => vec![],
        }
    }

    /// Searches the posts' titles, as Enter in the forum's search does; an
    /// empty query shows the posts again.
    pub(super) fn search_posts(&mut self, query: String) -> Vec<Effect> {
        match self.state.forum.as_mut() {
            Some(forum) => forum.list.search(query),
            None => vec![],
        }
    }

    /// Wants another page of cards, as the list does when scrolled near its
    /// end: their first messages, and older posts once the active ones run
    /// out.
    pub(super) fn more_posts(&mut self) -> Vec<Effect> {
        let active = self.forum_posts().map_or(0, |posts| posts.active.len());
        let Some(forum) = self.state.forum.as_mut() else {
            return vec![];
        };
        forum.shown += CARDS;
        forum.stalled = false;
        let mut effects = vec![];
        if forum.shown > active {
            effects.extend(forum.list.more());
        }
        effects.extend(self.ask_starters());
        effects
    }

    /// Takes a page of posts for the forum, if it's the one it asked for,
    /// with their first messages.
    pub(super) fn forum_page_came(
        &mut self,
        room: String,
        query: Option<String>,
        offset: u32,
        result: Result<(Vec<ListedThread>, bool), String>,
        starters: PostStarters,
    ) -> Vec<Effect> {
        let Some(forum) = self
            .state
            .forum
            .as_mut()
            .filter(|forum| forum.list.room == room)
        else {
            return vec![];
        };
        forum.starters.extend(starters);
        forum.list.came(query, offset, result);
        self.ask_starters()
    }

    /// Takes first messages for the forum's cards, and asks for the next
    /// ones they want.
    pub(super) fn posts_came(
        &mut self,
        room: String,
        result: Result<PostStarters, String>,
    ) -> Vec<Effect> {
        let Some(forum) = self
            .state
            .forum
            .as_mut()
            .filter(|forum| forum.list.room == room)
        else {
            return vec![];
        };
        let asked = std::mem::take(&mut forum.asking);
        match result {
            Ok(starters) => {
                // One Discord didn't answer for was deleted.
                for post in asked {
                    forum.starters.entry(post).or_insert(None);
                }
                forum.starters.extend(starters);
            }
            Err(error) => {
                tracing::debug!(%error, "a forum's first messages didn't come");
                forum.stalled = true;
            }
        }
        self.ask_starters()
    }

    /// Asks for the first messages the cards on top want, unless some are
    /// on their way: the posts the server's snapshot knows, as archived
    /// pages and searches bring their own.
    pub(super) fn ask_starters(&mut self) -> Vec<Effect> {
        let wanted: Vec<String> = match self.state.forum.as_ref() {
            Some(forum) if forum.asking.is_empty() && !forum.stalled => {
                let posts = self.forum_posts().unwrap_or_default();
                posts
                    .active
                    .iter()
                    .chain(&posts.older)
                    .take(forum.shown)
                    .filter(|post| post.starter.is_none())
                    .take(STARTERS_PER_ASK)
                    .map(|post| post.thread.room.clone())
                    .collect()
            }
            _ => return vec![],
        };
        let Some(forum) = self.state.forum.as_mut().filter(|_| !wanted.is_empty()) else {
            return vec![];
        };
        forum.asking = wanted.clone();
        vec![Effect::Backend(Command::Discord(DiscordCommand::Posts {
            room: forum.list.room.clone(),
            posts: wanted,
        }))]
    }

    /// The forum's posts as it shows them, while one shows.
    pub fn forum_posts(&self) -> Option<ForumPosts> {
        let forum = self.state.forum()?;
        let info = self.forum_info(&forum.list.room).unwrap_or_default();
        let sections = self.sections(&forum.list);
        let post = |thread: &ListedThread| ForumPost {
            tags: info
                .tags
                .iter()
                .filter(|tag| thread.tags.contains(&tag.id))
                .cloned()
                .collect(),
            starter: forum.starters.get(&thread.room).cloned(),
            new: !thread.archived && !thread.opened && created(thread) > forum.read,
            thread: thread.clone(),
        };
        let mut active: Vec<_> = sections
            .joined
            .iter()
            .chain(&sections.other)
            .map(post)
            .collect();
        let shown = self.forum_shown(&forum.list.room, &info);
        let order = |post: &ForumPost| match shown.by_creation {
            true => created(&post.thread),
            false => post.thread.last,
        };
        // Tags chosen show the posts with some or all of them, as Discord's
        // filter does.
        let fits = |post: &ForumPost| {
            let has = |tag: &String| post.thread.tags.contains(tag);
            shown.tags.is_empty()
                || if shown.match_all {
                    shown.tags.iter().all(has)
                } else {
                    shown.tags.iter().any(has)
                }
        };
        active.retain(fits);
        active.sort_by_key(|post| (!post.thread.pinned, std::cmp::Reverse(order(post))));
        let found = forum.list.search.as_ref().map(|search| {
            search.found.as_ref().map(|found| {
                found
                    .clone()
                    .map(|found| found.iter().map(post).filter(fits).collect())
            })
        });
        let mut older: Vec<_> = sections.older.iter().map(post).filter(fits).collect();
        if shown.by_creation {
            older.sort_by_key(|post| std::cmp::Reverse(created(&post.thread)));
        }
        Some(ForumPosts {
            active,
            older,
            found,
        })
    }

    /// What the forum, by room id, says of its posts.
    pub fn forum_info(&self, room: &str) -> Option<ForumInfo> {
        #[cfg(feature = "discord")]
        if let Some((server, Some(channel), _)) = self.discord_place(room)
            && let Some(forum) = server
                .server
                .channel(channel)
                .and_then(|channel| channel.forum.as_deref())
        {
            return Some(ForumInfo {
                tags: discord::forum_tags(forum),
                by_creation: forum.by_creation,
                require_tag: forum.require_tag,
                reaction: forum.reaction.as_ref().map(discord::shown_emoji),
                gallery: forum.gallery,
                media: forum.media,
                match_all: forum.match_all,
            });
        }
        let _ = room;
        None
    }

    /// How a forum, by room id, shows its posts now: as chosen, else as
    /// it says by default. A media channel's only show as a gallery.
    pub fn forum_shown(&self, room: &str, info: &ForumInfo) -> ForumShown {
        let choice = self.state.forum_choices.get(room);
        let pick =
            |chosen: Option<Option<bool>>, default: bool| chosen.flatten().unwrap_or(default);
        ForumShown {
            by_creation: pick(choice.map(|choice| choice.by_creation), info.by_creation),
            gallery: info.media || pick(choice.map(|choice| choice.gallery), info.gallery),
            match_all: pick(choice.map(|choice| choice.match_all), info.match_all),
            tags: choice
                .map(|choice| {
                    choice
                        .tags
                        .iter()
                        .filter(|tag| info.tags.iter().any(|known| known.id == **tag))
                        .cloned()
                        .collect()
                })
                .unwrap_or_default(),
        }
    }

    /// Sort & View, or a tag, changed for the forum on screen, as Discord's
    /// keeps it for each forum while it runs.
    pub(super) fn choose_forum(&mut self, change: ForumChange) -> Vec<Effect> {
        let Some(room) = self.state.forum().map(|forum| forum.list.room.clone()) else {
            return vec![];
        };
        let choice = self.state.forum_choices.entry(room).or_default();
        match change {
            ForumChange::Sort { by_creation } => choice.by_creation = Some(by_creation),
            ForumChange::Layout { gallery } => choice.gallery = Some(gallery),
            ForumChange::Matching { all } => choice.match_all = Some(all),
            ForumChange::Tag(tag) => {
                if let Some(at) = choice.tags.iter().position(|known| *known == tag) {
                    choice.tags.remove(at);
                } else {
                    choice.tags.push(tag);
                }
            }
            ForumChange::ClearTags => choice.tags.clear(),
            ForumChange::Reset => {
                let tags = std::mem::take(&mut choice.tags);
                *choice = ForumChoice {
                    tags,
                    ..ForumChoice::default()
                };
            }
        }
        vec![]
    }

    /// The last message read in a forum, by room id, as a snowflake.
    fn forum_read(&self, room: &str) -> u64 {
        #[cfg(feature = "discord")]
        if let Some((server, Some(channel), _)) = self.discord_place(room)
            && let Some(channel) = server.channel(channel)
        {
            return channel.marks.last_read;
        }
        let _ = room;
        0
    }
}

/// What Sort & View, or the tags over a forum, change.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ForumChange {
    /// Recently Active, or Date Posted.
    Sort {
        by_creation: bool,
    },
    /// List, or Gallery.
    Layout {
        gallery: bool,
    },
    /// Match Some, or Match All.
    Matching {
        all: bool,
    },
    /// Filter by a tag, by id, or stop.
    Tag(String),
    ClearTags,
    /// Reset to default: the forum's own sort, layout and matching.
    Reset,
}

/// When a post was made, as a snowflake: its room id's.
fn created(thread: &ListedThread) -> u64 {
    thread
        .room
        .rsplit(':')
        .next()
        .and_then(|id| id.parse().ok())
        .unwrap_or_default()
}
