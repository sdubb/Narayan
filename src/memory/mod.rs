pub mod consolidation;
pub mod embeddings;
pub mod store;
pub mod vector;

pub use consolidation::{apply_consolidation_metadata, MemoryConsolidator};
pub use embeddings::{build_embedding_model, EmbeddingModel};
pub use vector::{DistanceMetric, PgVectorStore, VectorDocument, VectorStore};
