//! Storage writer factory with automatic feature-based registration.
use crate::core::StorageWriter;
use anyhow::{anyhow, Result};
use std::collections::HashMap;

#[cfg(feature = "storage_csv")]
use crate::data::storage::CsvWriter;
#[cfg(feature = "storage_hdf5")]
use crate::data::storage::Hdf5Writer;
#[cfg(feature = "storage_arrow")]
use crate::data::storage::ArrowWriter;

type WriterFactory = Box<dyn Fn() -> Box<dyn StorageWriter> + Send + Sync>;

/// Registry for storage writer factories with automatic feature detection.
///
/// The registry automatically registers available storage writers based on
/// enabled Cargo features. This follows the Open/Closed Principle - new
/// storage formats can be added without modifying existing code.
///
/// # Examples
///
/// ```
/// use rust_daq::data::storage_factory::StorageWriterRegistry;
///
/// let registry = StorageWriterRegistry::new();
///
/// // List available formats (determined by enabled features)
/// let formats = registry.list_formats();
/// for format in formats {
///     println!("Available format: {}", format);
/// }
///
/// // Create a writer for a specific format
/// # #[cfg(feature = "storage_csv")]
/// let writer = registry.create("csv")?;
/// # Ok::<(), anyhow::Error>(())
/// ```
pub struct StorageWriterRegistry {
    factories: HashMap<String, WriterFactory>,
}

impl Default for StorageWriterRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl StorageWriterRegistry {
    /// Creates a new registry and automatically registers all available writers.
    ///
    /// Writers are registered based on enabled Cargo features:
    /// - `storage_csv` → CSV writer
    /// - `storage_hdf5` → HDF5 writer
    /// - `storage_arrow` → Arrow/Parquet writer
    pub fn new() -> Self {
        let mut registry = Self {
            factories: HashMap::new(),
        };

        // Automatically register writers based on enabled features
        #[cfg(feature = "storage_csv")]
        registry.register("csv", || Box::new(CsvWriter::new()));

        #[cfg(feature = "storage_hdf5")]
        registry.register("hdf5", || Box::new(Hdf5Writer::new()));

        #[cfg(feature = "storage_arrow")]
        registry.register("arrow", || Box::new(ArrowWriter::new()));

        registry
    }

    /// Registers a custom storage writer factory.
    ///
    /// This allows plugins or extensions to add their own storage formats
    /// at runtime.
    ///
    /// # Arguments
    ///
    /// * `format` - Format identifier (e.g., "csv", "custom_db")
    /// * `factory` - Closure that creates a new writer instance
    ///
    /// # Examples
    ///
    /// ```
    /// use rust_daq::data::storage_factory::StorageWriterRegistry;
    /// # use rust_daq::data::storage::CsvWriter;
    ///
    /// let mut registry = StorageWriterRegistry::new();
    /// # #[cfg(feature = "storage_csv")]
    /// registry.register("custom", || Box::new(CsvWriter::new()));
    /// ```
    pub fn register<F>(&mut self, format: &str, factory: F)
    where
        F: Fn() -> Box<dyn StorageWriter> + Send + Sync + 'static,
    {
        self.factories
            .insert(format.to_string(), Box::new(factory));
    }

    /// Creates a storage writer for the specified format.
    ///
    /// # Arguments
    ///
    /// * `format` - Format identifier (e.g., "csv", "hdf5", "arrow")
    ///
    /// # Returns
    ///
    /// Returns a boxed `StorageWriter` on success, or an error if the
    /// format is not registered (feature not enabled or unknown format).
    ///
    /// # Errors
    ///
    /// Returns `Err` if:
    /// - Format is not registered (feature not enabled)
    /// - Format name is invalid/unknown
    ///
    /// # Examples
    ///
    /// ```
    /// use rust_daq::data::storage_factory::StorageWriterRegistry;
    ///
    /// let registry = StorageWriterRegistry::new();
    ///
    /// # #[cfg(feature = "storage_csv")]
    /// match registry.create("csv") {
    ///     Ok(writer) => println!("CSV writer created"),
    ///     Err(e) => eprintln!("Failed to create writer: {}", e),
    /// }
    ///
    /// // Trying to create a writer for a disabled feature will fail
    /// # #[cfg(not(feature = "storage_hdf5"))]
    /// assert!(registry.create("hdf5").is_err());
    /// # Ok::<(), anyhow::Error>(())
    /// ```
    pub fn create(&self, format: &str) -> Result<Box<dyn StorageWriter>> {
        self.factories
            .get(format)
            .map(|factory| factory())
            .ok_or_else(|| {
                let available = self.list_formats().join(", ");
                anyhow!(
                    "Unsupported storage format: '{}'. Available formats: [{}]",
                    format,
                    available
                )
            })
    }

    /// Returns a list of all registered storage format names.
    ///
    /// The returned list depends on which Cargo features are enabled.
    ///
    /// # Returns
    ///
    /// Vector of format identifiers that can be passed to `create()`.
    ///
    /// # Examples
    ///
    /// ```
    /// use rust_daq::data::storage_factory::StorageWriterRegistry;
    ///
    /// let registry = StorageWriterRegistry::new();
    /// let formats = registry.list_formats();
    ///
    /// // Print all available formats
    /// for format in formats {
    ///     println!("Available: {}", format);
    /// }
    /// ```
    pub fn list_formats(&self) -> Vec<String> {
        let mut formats: Vec<String> = self.factories.keys().cloned().collect();
        formats.sort();
        formats
    }

    /// Checks if a specific format is available.
    ///
    /// # Arguments
    ///
    /// * `format` - Format identifier to check
    ///
    /// # Returns
    ///
    /// `true` if the format is registered and can be created, `false` otherwise.
    ///
    /// # Examples
    ///
    /// ```
    /// use rust_daq::data::storage_factory::StorageWriterRegistry;
    ///
    /// let registry = StorageWriterRegistry::new();
    ///
    /// # #[cfg(feature = "storage_csv")]
    /// assert!(registry.is_available("csv"));
    ///
    /// # #[cfg(not(feature = "storage_hdf5"))]
    /// assert!(!registry.is_available("hdf5"));
    /// ```
    pub fn is_available(&self, format: &str) -> bool {
        self.factories.contains_key(format)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_registry_creates_successfully() {
        let registry = StorageWriterRegistry::new();
        // Should have at least one format registered (CSV is in default features)
        assert!(!registry.list_formats().is_empty());
    }

    #[test]
    #[cfg(feature = "storage_csv")]
    fn test_csv_writer_available_with_feature() {
        let registry = StorageWriterRegistry::new();
        assert!(registry.is_available("csv"));
        assert!(registry.create("csv").is_ok());
    }

    #[test]
    fn test_invalid_format_returns_error() {
        let registry = StorageWriterRegistry::new();
        assert!(registry.create("nonexistent_format").is_err());
    }

    #[test]
    fn test_list_formats_is_sorted() {
        let registry = StorageWriterRegistry::new();
        let formats = registry.list_formats();
        let mut sorted_formats = formats.clone();
        sorted_formats.sort();
        assert_eq!(formats, sorted_formats);
    }

    #[test]
    fn test_custom_registration() {
        let mut registry = StorageWriterRegistry::new();
        let original_count = registry.list_formats().len();

        // Register a custom format using CSV writer as a stand-in
        #[cfg(feature = "storage_csv")]
        {
            registry.register("custom", || Box::new(CsvWriter::new()));
            assert_eq!(registry.list_formats().len(), original_count + 1);
            assert!(registry.is_available("custom"));
        }
    }
}
