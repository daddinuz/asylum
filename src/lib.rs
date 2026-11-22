//! String interning with scoped and process-lifetime storage.
//!
//! Use [`asylum::with`](crate::with) to intern temporary text in a scope or
//! [`asylum::intern`](crate::intern) to retain text for the life of the program.
//! [`asylum::intern_static`](crate::intern_static) can store static input without
//! copying its bytes.

use std::fmt::{self, Debug, Display, Formatter};
use std::hash::{BuildHasher, Hash, Hasher};
use std::marker::PhantomData;
use std::sync::{LazyLock, Mutex};

use hashbrown::{DefaultHashBuilder, HashTable};

/// An interned string.
///
/// Interning the same text twice in one interner yields equal symbols.
/// Comparing two symbols or hashing a symbol uses the entry's identity and
/// takes constant time. Comparing a symbol to a `&str`, `String`, or
/// `&String` on the right of `==` compares the text.
///
/// Direct comparison and collections of symbols require symbols from the same
/// interner. Each call to [`asylum::with`](crate::with) or
/// [`asylum::with_shards`](crate::with_shards) creates a separate interner.
/// [`asylum::intern`](crate::intern) and [`asylum::intern_static`](crate::intern_static)
/// use the same global interner.
/// Use [`Symbol::as_str`] to compare text across interners.
///
/// # Examples
///
/// Two scoped interners produce incompatible symbols even for identical text.
///
/// ```compile_fail
/// asylum::with(|outer| {
///     let first = outer.intern("text");
///     asylum::with(|inner| {
///         let second = inner.intern("text");
///         assert_eq!(first, second);
///     });
/// });
/// ```
///
/// The global interner and a scoped interner also produce distinct symbol types.
///
/// ```compile_fail
/// let global = asylum::intern("text");
/// asylum::with(|interner| {
///     let scoped = interner.intern("text");
///     let _symbols = [global, scoped];
/// });
/// ```
#[derive(Clone, Copy)]
pub struct Symbol<'a> {
    text: &'a Text,
    #[allow(unused)]
    marker: InvariantLifetime<'a>,
}

const _: () = assert!(std::mem::size_of::<usize>() == std::mem::size_of::<Symbol>());

impl<'a> Symbol<'a> {
    /// Returns the text for the lifetime of the interned entry.
    ///
    /// Text borrowed from a scoped symbol must stay within the callback.
    ///
    /// ```compile_fail
    /// let escaped: &str =
    ///     asylum::with(|interner| interner.intern("scoped").as_str());
    /// ```
    #[inline]
    pub const fn as_str(&self) -> &'a str {
        self.text.as_str()
    }
}

impl AsRef<str> for Symbol<'_> {
    #[inline]
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl PartialEq<&str> for Symbol<'_> {
    #[inline]
    fn eq(&self, other: &&str) -> bool {
        self.as_str() == *other
    }
}

impl PartialEq<&String> for Symbol<'_> {
    #[inline]
    fn eq(&self, other: &&String) -> bool {
        self.as_str() == other.as_str()
    }
}

impl PartialEq<String> for Symbol<'_> {
    #[inline]
    fn eq(&self, other: &String) -> bool {
        self.as_str() == other.as_str()
    }
}

impl PartialEq for Symbol<'_> {
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        std::ptr::eq(self.text, other.text)
    }
}

impl Eq for Symbol<'_> {}

impl Hash for Symbol<'_> {
    #[inline]
    fn hash<H: Hasher>(&self, state: &mut H) {
        std::ptr::hash(self.text, state);
    }
}

impl Debug for Symbol<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_tuple("Symbol").field(&self.text.as_str()).finish()
    }
}

impl Display for Symbol<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        Display::fmt(self.as_str(), f)
    }
}

// ---

/// A scoped interner provided by [`asylum::with`](crate::with) or
/// [`asylum::with_shards`](crate::with_shards).
///
/// Shared references to the interner support concurrent calls to [`Interner::intern`]
/// within the scope.
pub struct Interner<'id, const SHARDS: usize = DEFAULT_SHARD_COUNT> {
    registry: Registry<'id, SHARDS>,
    entries: &'id boxcar::Vec<Text>,
    #[allow(unused)]
    marker: InvariantLifetime<'id>,
}

impl<'id, const SHARDS: usize> Interner<'id, SHARDS> {
    /// Interns text for the current scope.
    ///
    /// A new entry copies the input text, so the symbol can outlive the input.
    /// Later calls with equal text reuse the same entry.
    pub fn intern(&self, s: &str) -> Symbol<'id> {
        let text = self.registry.get_or_insert_with(s, || {
            let index = self.entries.push(Text::Owned(s.into()));
            &self.entries[index]
        });

        Symbol {
            text,
            marker: InvariantLifetime::new(),
        }
    }

    /// Returns the number of distinct strings interned in this scope.
    pub fn size(&self) -> usize {
        self.registry.size()
    }

    /// Returns a guaranteed entry count the lookup tables can hold without reallocating.
    ///
    /// This is the current size plus the least remaining room in any shard. The
    /// storage for interned text grows separately.
    pub fn capacity(&self) -> usize {
        self.registry.capacity()
    }

    /// Shrinks each lookup table to fit its current entries.
    ///
    /// All entries stay interned and existing symbols remain valid. Interning
    /// can continue in other shards while each table is being shrunk.
    pub fn shrink_to_fit(&self) {
        self.registry.shrink_to_fit();
    }
}

/// Runs a callback with a fresh interner that has eight shards.
///
/// Scoped symbols remain valid throughout the callback, even if the interner
/// handle is dropped early. The entries are freed when the callback returns
/// or unwinds. The callback can return owned data or static symbols.
///
/// # Examples
///
/// Return owned data to use the text outside the scope.
///
/// ```
/// let text = asylum::with(|interner| {
///     let input = String::from("request");
///     let symbol = interner.intern(&input);
///     assert_eq!(symbol, interner.intern("request"));
///     symbol.as_str().to_owned()
/// });
/// assert_eq!(text, "request");
/// ```
///
/// A scoped symbol borrows its entry and cannot escape the callback.
///
/// ```compile_fail
/// use asylum::Symbol;
///
/// let escaped: Symbol<'static> =
///     asylum::with(|interner| interner.intern("scoped"));
/// ```
pub fn with<F, R>(f: F) -> R
where
    F: for<'scope> FnOnce(Interner<'scope>) -> R,
{
    with_shards::<DEFAULT_SHARD_COUNT, R>(f)
}

/// Runs a callback with a fresh interner that has `SHARDS` shards.
///
/// Scoped entries are freed when the callback returns or unwinds.
///
/// The shard array is stored inline. The compiler enforces a shard count from
/// 1 through 128.
///
/// ```compile_fail,E0080
/// asylum::with_shards::<0, _>(|_| ());
/// ```
///
/// ```compile_fail,E0080
/// asylum::with_shards::<129, _>(|_| ());
/// ```
///
/// # Example
///
/// ```
/// asylum::with_shards::<4, _>(|interner| {
///     let name = interner.intern("request");
///     assert_eq!(name, interner.intern("request"));
/// });
/// ```
pub fn with_shards<const SHARDS: usize, R>(
    f: impl for<'scope> FnOnce(Interner<'scope, SHARDS>) -> R,
) -> R {
    let entries = boxcar::Vec::new();
    f(Interner {
        registry: Registry::new(),
        entries: &entries,
        marker: InvariantLifetime::new(),
    })
}

// ---

static REGISTRY: LazyLock<Registry<'static, DEFAULT_SHARD_COUNT>> = LazyLock::new(Registry::new);

/// Interns text in the global interner.
///
/// A new entry copies the input and stays allocated for the life of the program.
/// Later calls with equal text reuse that entry, including calls to [`intern_static`].
/// Use [`with`] if the text should be freed at the end of a scope.
///
/// # Example
///
/// ```
/// let input = String::from("request");
/// let symbol = asylum::intern(&input);
/// drop(input);
///
/// let text: &'static str = symbol.as_str();
/// assert_eq!(text, "request");
/// ```
pub fn intern(s: &str) -> Symbol<'static> {
    intern_global(s, || Text::Owned(s.into()))
}

/// Interns static text in the global interner.
///
/// A new entry borrows the input bytes and stays allocated for the life of the
/// program. If [`intern`] or `intern_static` already stored equal text, the
/// existing entry is reused.
///
/// # Example
///
/// ```
/// let keyword = asylum::intern_static("let");
/// assert_eq!(keyword, asylum::intern("let"));
/// ```
pub fn intern_static(s: &'static str) -> Symbol<'static> {
    intern_global(s, || Text::Static(s))
}

fn intern_global(s: &str, create: impl FnOnce() -> Text) -> Symbol<'static> {
    let text = REGISTRY.get_or_insert_with(s, || Box::leak(Box::new(create())));

    Symbol {
        text,
        marker: InvariantLifetime::new(),
    }
}

/// Returns the number of distinct strings interned globally.
pub fn size() -> usize {
    REGISTRY.size()
}

/// Returns a guaranteed entry count the global lookup tables can hold without reallocating.
///
/// This is the current size plus the least remaining room in any shard. The
/// storage for interned text grows separately.
pub fn capacity() -> usize {
    REGISTRY.capacity()
}

/// Shrinks each global lookup table to fit its current entries.
///
/// All entries stay interned and existing symbols remain valid. Interning
/// can continue in other shards while each table is being shrunk.
pub fn shrink_to_fit() {
    REGISTRY.shrink_to_fit();
}

// ---

const DEFAULT_SHARD_COUNT: usize = 8;
const MAX_SHARD_COUNT: usize = 128;

struct Registry<'id, const SHARDS: usize> {
    shards: [Mutex<HashTable<&'id Text>>; SHARDS],
    hash_builder: DefaultHashBuilder,
}

impl<'id, const SHARDS: usize> Registry<'id, SHARDS> {
    fn new() -> Self {
        const {
            assert!(SHARDS > 0, "an interner needs at least one shard");
            assert!(
                SHARDS <= MAX_SHARD_COUNT,
                "an interner supports at most 128 shards"
            );
        };

        Self {
            shards: std::array::from_fn(|_| Mutex::new(HashTable::new())),
            hash_builder: DefaultHashBuilder::default(),
        }
    }

    fn get_or_insert_with(&self, text: &str, create: impl FnOnce() -> &'id Text) -> &'id Text {
        let hash = self.hash_builder.hash_one(text);
        // Leave the lower hash bits for bucket selection within the shard.
        let index = (hash >> 32) as usize % SHARDS;
        let mut shard = self.shards[index].lock().unwrap();

        if let Some(&entry) = shard.find(hash, |entry| entry.as_str() == text) {
            return entry;
        }

        let entry = create();
        shard.insert_unique(hash, entry, |entry| {
            self.hash_builder.hash_one(entry.as_str())
        });

        entry
    }

    fn size(&self) -> usize {
        self.shards
            .iter()
            .map(|shard| shard.lock().unwrap().len())
            .sum()
    }

    fn capacity(&self) -> usize {
        // Hold all shards in index order for a consistent capacity snapshot.
        let mut guards = [const { None }; SHARDS];
        for (slot, shard) in guards.iter_mut().zip(&self.shards) {
            match shard.lock() {
                Ok(guard) => *slot = Some(guard),
                Err(error) => {
                    drop(error);
                    // Release healthy shards before unwinding so they stay healthy.
                    drop(guards);
                    panic!("interner shard mutex is poisoned");
                }
            }
        }

        let size: usize = guards.iter().flatten().map(|shard| shard.len()).sum();
        let remaining = guards
            .iter()
            .flatten()
            .map(|shard| shard.capacity() - shard.len())
            .min()
            .unwrap();

        size + remaining
    }

    fn shrink_to_fit(&self) {
        for shard in &self.shards {
            shard
                .lock()
                .unwrap()
                .shrink_to_fit(|entry| self.hash_builder.hash_one(entry.as_str()));
        }
    }
}

// ---

// Table keys compare and hash by text. Symbols use the address of the key.
enum Text {
    Owned(Box<str>),
    Static(&'static str),
}

impl Text {
    const fn as_str(&self) -> &str {
        match self {
            Self::Owned(s) => s,
            Self::Static(s) => s,
        }
    }
}

// ---

#[derive(Clone, Copy)]
struct InvariantLifetime<'a>(PhantomData<fn(&'a ()) -> &'a ()>);

impl InvariantLifetime<'_> {
    const fn new() -> Self {
        Self(PhantomData)
    }
}

#[cfg(test)]
#[path = "../tests/registry/poisoning.rs"]
mod poisoning_tests;
