// src/adapters/command_batch.rs

use anyhow::Result;
use async_trait::async_trait;

/// A trait for executing a batch of commands.
#[async_trait]
pub trait BatchExecutor {
    /// Flushes the command batch, sending all queued commands.
    async fn flush(&mut self, commands: &[String]) -> Result<()>;
}

/// A struct for batching commands to be sent to an instrument.
pub struct CommandBatch<'a, E: BatchExecutor> {
    executor: &'a mut E,
    commands: Vec<String>,
}

impl<'a, E: BatchExecutor> CommandBatch<'a, E> {
    /// Creates a new `CommandBatch`.
    pub fn new(executor: &'a mut E) -> Self {
        Self {
            executor,
            commands: Vec::new(),
        }
    }

    /// Adds a command to the batch.
    pub fn queue(&mut self, command: String) {
        self.commands.push(command);
    }

    /// Returns the commands in the batch.
    pub fn commands(&self) -> &Vec<String> {
        &self.commands
    }

    /// Flushes the command batch.
    pub async fn flush(&mut self) -> Result<()> {
        self.executor.flush(&self.commands).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;

    struct MockExecutor {
        flushed_commands: Vec<String>,
    }

    #[async_trait]
    impl BatchExecutor for MockExecutor {
        async fn flush(&mut self, batch: &CommandBatch<Self>) -> Result<()> {
            self.flushed_commands = batch.commands().clone();
            Ok(())
        }
    }

    #[tokio::test]
    async fn test_batch_queue_and_flush() {
        let mut executor = MockExecutor {
            flushed_commands: Vec::new(),
        };
        let mut batch = CommandBatch::new(&mut executor);

        batch.queue("CMD1".to_string());
        batch.queue("CMD2".to_string());

        assert_eq!(batch.commands(), &vec!["CMD1", "CMD2"]);

        batch.flush().await.unwrap();

        assert_eq!(executor.flushed_commands, vec!["CMD1", "CMD2"]);
    }
}
