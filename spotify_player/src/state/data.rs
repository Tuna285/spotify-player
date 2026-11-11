use std::io::{BufReader, BufWriter};
use std::{
    collections::{HashMap, HashSet},
    path::Path,
};

use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::sync::LazyLock;

use super::model::{
    Album, Artist, Category, Context, ContextId, ContextSortState, Id, Playlist,
    PlaylistFolderItem, PlaylistFolderNode, SearchResults, Show, Track,
};
use super::Lyrics;

pub type DataReadGuard<'a> = parking_lot::RwLockReadGuard<'a, AppData>;

#[derive(Debug, Copy, Clone)]
pub enum FileCacheKey {
    Playlists,
    PlaylistFolders,
    FollowedArtists,
    SavedShows,
    SavedAlbums,
    SavedTracks,
    RecentlyPlayedTracks,
    TopTracks,
    ContextSortOrders,
}

/// default time-to-live cache duration
pub static TTL_CACHE_DURATION: LazyLock<std::time::Duration> =
    LazyLock::new(|| std::time::Duration::from_hours(1));

/// the application's data
pub struct AppData {
    pub user_data: UserData,
    pub caches: MemoryCaches,
    pub browse: BrowseData,
}

#[derive(Debug)]
/// current user's data
pub struct UserData {
    pub user: Option<rspotify::model::PrivateUser>,
    pub playlists: Vec<PlaylistFolderItem>,
    pub playlist_folder_node: Option<PlaylistFolderNode>,
    pub followed_artists: Vec<Artist>,
    pub saved_shows: Vec<Show>,
    pub saved_albums: Vec<Album>,
    pub saved_tracks: HashMap<String, Track>,
    pub recently_played_tracks: Vec<Track>,
    pub top_tracks: Vec<Track>,
    pub context_sort_orders: HashMap<String, ContextSortState>,
}

#[derive(Debug)]
pub enum SearchCacheEntry {
    Loading,
    Ready(SearchResults),
    Failed,
}

/// the application's in-memory caches
pub struct MemoryCaches {
    pub context: ttl_cache::TtlCache<String, Context>,
    pub loading_contexts: HashSet<String>,
    pub search: ttl_cache::TtlCache<String, SearchCacheEntry>,
    pub lyrics: ttl_cache::TtlCache<String, Option<Lyrics>>,
    pub genres: ttl_cache::TtlCache<String, Vec<String>>,
    #[cfg(feature = "image")]
    pub images: ttl_cache::TtlCache<String, image::DynamicImage>,
}

#[derive(Default, Debug)]
/// Spotify browse data
pub struct BrowseData {
    pub categories: Option<Vec<Category>>,
    pub category_playlists: HashMap<String, Vec<Playlist>>,
}

impl MemoryCaches {
    pub fn new() -> Self {
        Self {
            context: ttl_cache::TtlCache::new(64),
            loading_contexts: HashSet::new(),
            search: ttl_cache::TtlCache::new(64),
            lyrics: ttl_cache::TtlCache::new(64),
            genres: ttl_cache::TtlCache::new(64),
            #[cfg(feature = "image")]
            images: ttl_cache::TtlCache::new(64),
        }
    }

    pub fn is_context_loading(&self, uri: &str) -> bool {
        self.loading_contexts.contains(uri)
    }

    pub fn begin_context_fetch(&mut self, uri: &str) -> bool {
        self.loading_contexts.insert(uri.to_string())
    }

    pub fn finish_context_fetch(&mut self, uri: &str) {
        self.loading_contexts.remove(uri);
    }

    pub fn begin_search(&mut self, query: &str) -> bool {
        let should_begin = matches!(
            self.search.get(query),
            None | Some(SearchCacheEntry::Failed)
        );

        if should_begin {
            self.search.insert(
                query.to_string(),
                SearchCacheEntry::Loading,
                *TTL_CACHE_DURATION,
            );
        }

        should_begin
    }

    pub fn complete_search(&mut self, query: String, results: SearchResults) {
        self.search
            .insert(query, SearchCacheEntry::Ready(results), *TTL_CACHE_DURATION);
    }

    pub fn fail_search(&mut self, query: String) {
        if matches!(self.search.get(&query), Some(SearchCacheEntry::Loading)) {
            self.search
                .insert(query, SearchCacheEntry::Failed, *TTL_CACHE_DURATION);
        }
    }

    pub fn search_results(&self, query: &str) -> Option<&SearchResults> {
        match self.search.get(query) {
            Some(SearchCacheEntry::Ready(results)) => Some(results),
            _ => None,
        }
    }
}

impl AppData {
    pub fn new(cache_folder: &Path) -> Self {
        Self {
            user_data: UserData::new_from_file_caches(cache_folder),
            caches: MemoryCaches::new(),
            browse: BrowseData::default(),
        }
    }

    /// Get a list of tracks inside a given context
    pub fn context_tracks_mut(&mut self, id: &ContextId) -> Option<&mut Vec<Track>> {
        let c = self.caches.context.get_mut(&id.uri())?;

        Some(match c {
            Context::Album { tracks, .. }
            | Context::Playlist { tracks, .. }
            | Context::Tracks { tracks, .. }
            | Context::Artist {
                top_tracks: tracks, ..
            } => tracks,
            Context::Show { .. } => {
                return None;
            }
        })
    }

    pub fn context_tracks(&self, id: &ContextId) -> Option<&Vec<Track>> {
        let c = self.caches.context.get(&id.uri())?;
        Some(match c {
            Context::Album { tracks, .. }
            | Context::Playlist { tracks, .. }
            | Context::Tracks { tracks, .. }
            | Context::Artist {
                top_tracks: tracks, ..
            } => tracks,
            Context::Show { .. } => {
                return None;
            }
        })
    }
}

impl UserData {
    /// Construct a new user data based on file caches
    pub fn new_from_file_caches(cache_folder: &Path) -> Self {
        Self {
            user: None,
            playlists: load_data_from_file_cache(FileCacheKey::Playlists, cache_folder)
                .unwrap_or_default(),
            playlist_folder_node: load_data_from_file_cache(
                FileCacheKey::PlaylistFolders,
                cache_folder,
            ),
            followed_artists: load_data_from_file_cache(
                FileCacheKey::FollowedArtists,
                cache_folder,
            )
            .unwrap_or_default(),
            saved_shows: load_data_from_file_cache(FileCacheKey::SavedShows, cache_folder)
                .unwrap_or_default(),
            saved_albums: load_data_from_file_cache(FileCacheKey::SavedAlbums, cache_folder)
                .unwrap_or_default(),
            saved_tracks: load_data_from_file_cache(FileCacheKey::SavedTracks, cache_folder)
                .unwrap_or_default(),
            recently_played_tracks: load_data_from_file_cache(
                FileCacheKey::RecentlyPlayedTracks,
                cache_folder,
            )
            .unwrap_or_default(),
            top_tracks: load_data_from_file_cache(FileCacheKey::TopTracks, cache_folder)
                .unwrap_or_default(),
            context_sort_orders: load_data_from_file_cache(
                FileCacheKey::ContextSortOrders,
                cache_folder,
            )
            .unwrap_or_default(),
        }
    }

    /// Get a list of playlist items that are **possibly** modifiable by user
    ///
    /// If `folder_id` is provided, returns items in the given folder id.
    /// Otherwise, returns the all items.
    pub fn modifiable_playlist_items(&self, folder_id: Option<usize>) -> Vec<&PlaylistFolderItem> {
        match self.user {
            None => vec![],
            Some(ref u) => self
                .playlists
                .iter()
                // filter items in a folder (if specified)
                .filter(|item| {
                    if let Some(folder_id) = folder_id {
                        match item {
                            PlaylistFolderItem::Playlist(p) => p.current_folder_id == folder_id,
                            PlaylistFolderItem::Folder(f) => f.current_id == folder_id,
                        }
                    } else {
                        true
                    }
                })
                // filter modifiable items
                .filter(|item| match item {
                    PlaylistFolderItem::Playlist(p) => p.owner.1 == u.id || p.collaborative,
                    PlaylistFolderItem::Folder(_) => true,
                })
                .collect(),
        }
    }

    /// Get playlists items for the given folder id
    pub fn folder_playlists_items(&self, folder_id: usize) -> Vec<&PlaylistFolderItem> {
        self.playlists
            .iter()
            .filter(|item| match item {
                PlaylistFolderItem::Playlist(p) => p.current_folder_id == folder_id,
                PlaylistFolderItem::Folder(f) => f.current_id == folder_id,
            })
            .collect()
    }

    /// Check if a track is a liked track
    pub fn is_liked_track(&self, track: &Track) -> bool {
        self.saved_tracks.contains_key(&track.id.uri())
    }

    /// Get the user's liked tracks by the given artist, sorted by liked date (newest first)
    pub fn liked_tracks_by_artist(&self, artist: &Artist) -> Vec<Track> {
        let mut tracks: Vec<Track> = self
            .saved_tracks
            .values()
            .filter(|t| t.artists.iter().any(|a| a.id == artist.id))
            .cloned()
            .collect();
        tracks.sort_by_key(|t| std::cmp::Reverse(t.added_at));
        tracks
    }

    /// Check if a playlist is followed
    pub fn is_followed_playlist(&self, playlist: &Playlist) -> bool {
        self.playlists.iter().any(|x| match x {
            PlaylistFolderItem::Playlist(p) => p.id == playlist.id,
            PlaylistFolderItem::Folder(_) => false,
        })
    }
}

pub fn store_data_into_file_cache<T: Serialize>(
    key: FileCacheKey,
    cache_folder: &Path,
    data: &T,
) -> std::io::Result<()> {
    let path = cache_folder.join(format!("{key:?}_cache.json"));
    let f = BufWriter::new(std::fs::File::create(path)?);
    serde_json::to_writer(f, data)?;
    Ok(())
}

pub fn load_data_from_file_cache<T>(key: FileCacheKey, cache_folder: &Path) -> Option<T>
where
    T: DeserializeOwned,
{
    let path = cache_folder.join(format!("{key:?}_cache.json"));
    if path.exists() {
        tracing::info!("Loading {key:?} data from {}...", path.display());
        let f = BufReader::new(std::fs::File::open(path).expect("path exists"));
        match serde_json::from_reader(f) {
            Ok(data) => {
                tracing::info!("Successfully loaded {key:?} data!");
                Some(data)
            }
            Err(err) => {
                tracing::error!("Failed to load {key:?} data: {err:#}");
                None
            }
        }
    } else {
        None
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedPlaylistTracks {
    pub snapshot_id: String,
    pub tracks: Vec<Track>,
}

pub fn load_playlist_tracks_cache(
    playlist_id: &str,
    snapshot_id: &str,
    cache_folder: &Path,
) -> Option<Vec<Track>> {
    if snapshot_id.is_empty() {
        return None;
    }
    let path = cache_folder
        .join("playlist_tracks")
        .join(format!("{playlist_id}.json"));
    if !path.exists() {
        return None;
    }
    let f = BufReader::new(std::fs::File::open(path).ok()?);
    let cached: CachedPlaylistTracks = serde_json::from_reader(f).ok()?;
    if cached.snapshot_id == snapshot_id {
        Some(cached.tracks)
    } else {
        None
    }
}

pub fn store_playlist_tracks_cache(
    playlist_id: &str,
    snapshot_id: String,
    tracks: Vec<Track>,
    cache_folder: &Path,
) -> std::io::Result<()> {
    let dir = cache_folder.join("playlist_tracks");
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("{playlist_id}.json"));
    let f = BufWriter::new(std::fs::File::create(path)?);
    serde_json::to_writer(f, &CachedPlaylistTracks { snapshot_id, tracks })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{MemoryCaches, SearchCacheEntry, SearchResults};
    use crate::state::model::{ContextSortState, Track, TrackId, TrackOrder};

    #[test]
    fn context_fetch_lifecycle_suppresses_duplicate_requests() {
        let mut caches = MemoryCaches::new();
        let uri = "spotify:playlist:6ZXsoRtyeBVYjaBkoT0MgM";

        assert!(!caches.is_context_loading(uri));
        assert!(caches.begin_context_fetch(uri));
        assert!(caches.is_context_loading(uri));
        assert!(!caches.begin_context_fetch(uri));

        caches.finish_context_fetch(uri);
        assert!(!caches.is_context_loading(uri));
        assert!(caches.begin_context_fetch(uri));
    }

    fn sample_track(id: &'static str, name: &str, added_at: u64) -> Track {
        Track {
            id: TrackId::from_id(id).unwrap().into_static(),
            name: name.to_string(),
            artists: vec![],
            album: None,
            duration: std::time::Duration::default(),
            explicit: false,
            added_at,
        }
    }

    #[test]
    fn context_sort_state_applies_and_serializes() {
        let mut tracks = vec![
            sample_track("3n3Ppam7vgaVa1iaRUc9Lp", "B", 10),
            sample_track("4uLU6hMCjMI75M1A2tKUQC", "A", 30),
            sample_track("1301WleyT98MSxVHPZCA6M", "C", 20),
        ];

        let state = ContextSortState {
            order: Some(TrackOrder::AddedAt),
            reversed: true,
        };
        assert!(state.is_active());
        state.apply(&mut tracks);

        assert_eq!(tracks[0].added_at, 30);
        assert_eq!(tracks[1].added_at, 20);
        assert_eq!(tracks[2].added_at, 10);

        let json = serde_json::to_string(&state).unwrap();
        let restored: ContextSortState = serde_json::from_str(&json).unwrap();
        assert_eq!(restored, state);
    }

    #[test]
    fn search_lifecycle_suppresses_duplicate_requests() {
        let mut caches = MemoryCaches::new();

        assert!(caches.begin_search("query"));
        assert!(matches!(
            caches.search.get("query"),
            Some(SearchCacheEntry::Loading)
        ));
        assert!(!caches.begin_search("query"));

        caches.complete_search("query".to_string(), SearchResults::default());

        assert!(caches.search_results("query").is_some());
        assert!(!caches.begin_search("query"));
    }

    #[test]
    fn failed_search_can_be_retried() {
        let mut caches = MemoryCaches::new();

        assert!(caches.begin_search("query"));
        caches.fail_search("query".to_string());

        assert!(matches!(
            caches.search.get("query"),
            Some(SearchCacheEntry::Failed)
        ));
        assert!(caches.begin_search("query"));
        assert!(matches!(
            caches.search.get("query"),
            Some(SearchCacheEntry::Loading)
        ));
    }
}
