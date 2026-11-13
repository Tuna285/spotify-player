use std::io::{BufReader, BufWriter};
use std::{
    collections::{HashMap, HashSet},
    path::Path,
};

use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::sync::LazyLock;

use super::model::{
    Album, Artist, Category, Context, ContextId, ContextSortState, Id, Playlist,
    PlaylistFolderItem, PlaylistFolderNode, RecentItem, SearchResults, Show, StationItem, Track,
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
    RecentlyPlayedItems,
    TopTracks,
    TopArtists,
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
    pub recently_played_items: Vec<RecentItem>,
    pub top_tracks: Vec<Track>,
    pub top_artists: Vec<Artist>,
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
            recently_played_items: load_data_from_file_cache(
                FileCacheKey::RecentlyPlayedItems,
                cache_folder,
            )
            .unwrap_or_default(),
            top_tracks: load_data_from_file_cache(FileCacheKey::TopTracks, cache_folder)
                .unwrap_or_default(),
            top_artists: load_data_from_file_cache(FileCacheKey::TopArtists, cache_folder)
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

    /// Get personal playlists (excluding Spotify Made For You mixes) for the given folder id
    pub fn personal_folder_playlists_items(&self, folder_id: usize) -> Vec<&PlaylistFolderItem> {
        self.folder_playlists_items(folder_id)
            .into_iter()
            .filter(|item| !matches!(item, PlaylistFolderItem::Playlist(p) if p.is_made_for_you()))
            .collect()
    }

    /// Get Spotify "Made For You" playlists saved in the user's library
    pub fn made_for_you_playlist_items(&self) -> Vec<&PlaylistFolderItem> {
        self.playlists
            .iter()
            .filter(|item| matches!(item, PlaylistFolderItem::Playlist(p) if p.is_made_for_you()))
            .collect()
    }

    /// Prepend an item into `recently_played_items`, deduplicating by key and capping at 50 entries
    pub fn push_recent_item(&mut self, item: RecentItem) {
        let key = item.cache_key();
        self.recently_played_items.retain(|i| i.cache_key() != key);
        self.recently_played_items.insert(0, item);
        self.recently_played_items.truncate(50);
    }

    /// Build diverse Recommended Stations from the user's favorite artists and top/liked tracks
    pub fn recommended_stations(&self) -> Vec<StationItem> {
        let mut seen_artists = HashSet::new();
        let mut artist_seeds = Vec::new();
        for artist in self.top_artists.iter().chain(self.followed_artists.iter()) {
            if seen_artists.insert(artist.id.clone()) {
                artist_seeds.push(StationItem::Artist(artist.clone()));
            }
        }

        // First pass over top tracks allows at most 2 tracks per primary artist for diversity;
        // fallback to saved tracks if top_tracks is empty.
        let track_source: Vec<&Track> = if !self.top_tracks.is_empty() {
            self.top_tracks.iter().collect()
        } else {
            self.saved_tracks.values().collect()
        };
        let mut per_artist_count: HashMap<String, usize> = HashMap::new();
        let mut track_seeds = Vec::new();
        let mut deferred_tracks = Vec::new();
        for &track in &track_source {
            let primary = track
                .artists
                .first()
                .map(|a| a.name.clone())
                .unwrap_or_default();
            let count = per_artist_count.entry(primary).or_insert(0);
            if *count < 2 {
                *count += 1;
                track_seeds.push(StationItem::Track(track.clone()));
            } else {
                deferred_tracks.push(StationItem::Track(track.clone()));
            }
        }
        track_seeds.extend(deferred_tracks);

        // Interleave ~2 artist stations per 1 track station, similar to Spotify's shelf
        let mut stations = Vec::with_capacity(30);
        let mut a_iter = artist_seeds.into_iter();
        let mut t_iter = track_seeds.into_iter();
        while stations.len() < 30 {
            let a1 = a_iter.next();
            let a2 = a_iter.next();
            let t1 = t_iter.next();
            if a1.is_none() && a2.is_none() && t1.is_none() {
                break;
            }
            if let Some(s) = a1 {
                stations.push(s);
            }
            if stations.len() < 30 {
                if let Some(s) = a2 {
                    stations.push(s);
                }
            }
            if stations.len() < 30 {
                if let Some(s) = t1 {
                    stations.push(s);
                }
            }
        }
        stations
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

    #[test]
    fn library_splits_made_for_you_and_builds_diverse_stations() {
        use super::{HashMap, UserData};
        use crate::state::model::{
            Artist, ArtistId, Playlist, PlaylistFolderItem, PlaylistId, RecentItem, StationItem,
            UserId,
        };

        let make_playlist = |id: &'static str, name: &str, owner: &'static str| {
            PlaylistFolderItem::Playlist(Playlist {
                id: PlaylistId::from_id(id).unwrap().into_static(),
                collaborative: false,
                name: name.to_string(),
                owner: (owner.to_string(), UserId::from_id(owner).unwrap().into_static()),
                desc: String::new(),
                current_folder_id: 0,
                snapshot_id: String::new(),
            })
        };
        let make_artist = |id: &'static str, name: &str| Artist {
            id: ArtistId::from_id(id).unwrap().into_static(),
            name: name.to_string(),
        };

        let mut user_data = UserData {
            user: None,
            playlists: vec![
                make_playlist("37i9dQZF1E351234567890", "Daily Mix 1", "spotify"),
                make_playlist("37i9dQZEVXc12345678901", "Discover Weekly", "spotify"),
                make_playlist("37i9dQZF1DXcBWIGoYBM5M", "Today's Top Hits", "spotify"),
                make_playlist("6ZXsoRtyeBVYjaBkoT0MgM", "asobi", "huynhtuan"),
            ],
            playlist_folder_node: None,
            followed_artists: vec![
                make_artist("4YRxDV8wJFPHPTeXepOstw", "Arijit"),
                make_artist("1vCWHaC5f2uS3yhpwWbIA6", "Avicii"),
            ],
            saved_shows: vec![],
            saved_albums: vec![],
            saved_tracks: HashMap::new(),
            recently_played_tracks: vec![],
            recently_played_items: vec![],
            top_tracks: vec![
                Track {
                    artists: vec![make_artist("4YRxDV8wJFPHPTeXepOstw", "Arijit")],
                    ..sample_track("3n3Ppam7vgaVa1iaRUc9Lp", "T1", 1)
                },
                Track {
                    artists: vec![make_artist("4YRxDV8wJFPHPTeXepOstw", "Arijit")],
                    ..sample_track("4uLU6hMCjMI75M1A2tKUQC", "T2", 2)
                },
                Track {
                    artists: vec![make_artist("4YRxDV8wJFPHPTeXepOstw", "Arijit")],
                    ..sample_track("1301WleyT98MSxVHPZCA6M", "T3", 3)
                },
                Track {
                    artists: vec![make_artist("1vCWHaC5f2uS3yhpwWbIA6", "Avicii")],
                    ..sample_track("0VjIjW4GlUZAMYd2vXMi3b", "T4", 4)
                },
            ],
            top_artists: vec![make_artist("4YRxDV8wJFPHPTeXepOstw", "Arijit")],
            context_sort_orders: HashMap::new(),
        };

        let personal = user_data.personal_folder_playlists_items(0);
        let made = user_data.made_for_you_playlist_items();
        assert_eq!(personal.len(), 2);
        assert_eq!(made.len(), 2);

        let stations = user_data.recommended_stations();
        // Should include deduplicated artists (Arijit, Avicii) and prioritize Avicii's T4 before Arijit's 3rd track T3
        assert!(matches!(&stations[0], StationItem::Artist(a) if a.name == "Arijit"));
        assert!(matches!(&stations[1], StationItem::Artist(a) if a.name == "Avicii"));
        let track_titles: Vec<&str> = stations
            .iter()
            .filter_map(|s| match s {
                StationItem::Track(t) => Some(t.name.as_str()),
                StationItem::Artist(_) => None,
            })
            .collect();
        assert_eq!(track_titles, vec!["T1", "T2", "T4", "T3"]);

        user_data.push_recent_item(RecentItem::Track(sample_track(
            "3n3Ppam7vgaVa1iaRUc9Lp",
            "T1",
            1,
        )));
        user_data.push_recent_item(RecentItem::Artist(make_artist(
            "1vCWHaC5f2uS3yhpwWbIA6",
            "Avicii",
        )));
        user_data.push_recent_item(RecentItem::Track(sample_track(
            "3n3Ppam7vgaVa1iaRUc9Lp",
            "T1",
            1,
        )));
        assert_eq!(user_data.recently_played_items.len(), 2);
        assert!(matches!(user_data.recently_played_items[0], RecentItem::Track(_)));
    }
}
