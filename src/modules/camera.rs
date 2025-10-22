//! Camera Module - Reference Implementation for Phase 3B
//!
//! Demonstrates type-safe runtime instrument assignment using the Camera meta trait.

use super::{Module, ModuleStatus};
use super::meta_instruments::Camera;
use async_trait::async_trait;
use anyhow::{anyhow, Result};
use std::any::Any;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tokio::time::Duration;

/// Camera module for image acquisition workflows.
///
/// This module demonstrates the Phase 3B module system design:
/// - Type-safe instrument assignment (only accepts Camera trait objects)
/// - Runtime reassignment support (hot-swap cameras)
/// - Async acquisition loop in separate task
/// - Integration with actor-based app via downcast
pub struct CameraModule {
    name: String,
    camera: Option<Arc<Mutex<Box<dyn Camera>>>>,
    running: bool,
    acquisition_task: Option<JoinHandle<()>>,
}

impl CameraModule {
    /// Creates a new camera module with the given name
    pub fn new(name: String) -> Self {
        Self {
            name,
            camera: None,
            running: false,
            acquisition_task: None,
        }
    }

    /// Type-safe camera assignment - only accepts Camera trait objects.
    ///
    /// This method enforces type safety at compile time: only instruments
    /// implementing the Camera trait can be passed here.
    ///
    /// # Errors
    ///
    /// Returns error if module is currently running. Stop the module before reassignment.
    pub fn assign_camera(&mut self, camera: Box<dyn Camera>) -> Result<()> {
        if self.running {
            return Err(anyhow!("Cannot assign camera while module is running"));
        }
        self.camera = Some(Arc::new(Mutex::new(camera)));
        log::info!("Camera assigned to module '{}'", self.name);
        Ok(())
    }

    /// Unassign the current camera from this module
    pub fn unassign_camera(&mut self) -> Result<()> {
        if self.running {
            return Err(anyhow!("Cannot unassign camera while module is running"));
        }
        self.camera = None;
        log::info!("Camera unassigned from module '{}'", self.name);
        Ok(())
    }

    /// Check if a camera is currently assigned
    pub fn has_camera(&self) -> bool {
        self.camera.is_some()
    }
}

#[async_trait]
impl Module for CameraModule {
    fn name(&self) -> &str {
        &self.name
    }

    async fn start(&mut self) -> Result<()> {
        let camera = self.camera.as_ref()
            .ok_or_else(|| anyhow!("No camera assigned to module"))?
            .clone();

        self.running = true;
        log::info!("Starting camera module '{}'", self.name);

        // Spawn acquisition loop
        let module_name = self.name.clone();
        let task = tokio::spawn(async move {
            loop {
                let mut cam = camera.lock().await;
                match cam.capture().await {
                    Ok(image) => {
                        log::debug!(
                            "Module '{}': Captured {}x{} image",
                            module_name,
                            image.width,
                            image.height
                        );
                        // TODO: Broadcast image data via module output stream
                    }
                    Err(e) => {
                        log::error!("Module '{}': Capture failed: {}", module_name, e);
                    }
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        });

        self.acquisition_task = Some(task);
        Ok(())
    }

    async fn stop(&mut self) -> Result<()> {
        log::info!("Stopping camera module '{}'", self.name);
        self.running = false;

        if let Some(task) = self.acquisition_task.take() {
            task.abort();
        }

        Ok(())
    }

    fn is_running(&self) -> bool {
        self.running
    }

    fn status(&self) -> ModuleStatus {
        if self.running {
            ModuleStatus::Running
        } else if self.camera.is_some() {
            ModuleStatus::Idle
        } else {
            ModuleStatus::Error("No camera assigned".to_string())
        }
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::ImageData;
    use crate::modules::meta_instruments::MetaInstrument;

    // Mock Camera for testing
    struct MockCamera {
        id: String,
        exposure_ms: f64,
    }

    impl MetaInstrument for MockCamera {
        fn instrument_id(&self) -> &str {
            &self.id
        }

        fn instrument_type(&self) -> &str {
            "camera"
        }

        fn capabilities(&self) -> Vec<String> {
            vec!["camera".to_string()]
        }
    }

    #[async_trait]
    impl Camera for MockCamera {
        async fn capture(&mut self) -> Result<ImageData> {
            Ok(ImageData {
                timestamp: chrono::Utc::now(),
                channel: format!("{}_image", self.id),
                width: 640,
                height: 480,
                pixels: vec![0.0; 640 * 480],
                unit: "counts".to_string(),
                metadata: None,
            })
        }

        async fn set_exposure(&mut self, ms: f64) -> Result<()> {
            self.exposure_ms = ms;
            Ok(())
        }

        async fn get_exposure(&self) -> Result<f64> {
            Ok(self.exposure_ms)
        }

        async fn set_roi(&mut self, _x: u32, _y: u32, _width: u32, _height: u32) -> Result<()> {
            Ok(())
        }

        async fn get_sensor_size(&self) -> Result<(u32, u32)> {
            Ok((640, 480))
        }
    }

    #[tokio::test]
    async fn test_camera_module_assignment() {
        let mut module = CameraModule::new("test".to_string());
        let mock_camera: Box<dyn Camera> = Box::new(MockCamera {
            id: "mock1".to_string(),
            exposure_ms: 100.0,
        });

        // Should succeed
        assert!(module.assign_camera(mock_camera).is_ok());
        assert!(module.has_camera());

        // Should fail while running
        module.start().await.unwrap();
        let another_camera: Box<dyn Camera> = Box::new(MockCamera {
            id: "mock2".to_string(),
            exposure_ms: 50.0,
        });
        assert!(module.assign_camera(another_camera).is_err());

        // Should succeed after stop
        module.stop().await.unwrap();
        let yet_another: Box<dyn Camera> = Box::new(MockCamera {
            id: "mock3".to_string(),
            exposure_ms: 200.0,
        });
        assert!(module.assign_camera(yet_another).is_ok());
    }

    #[test]
    fn test_module_status() {
        let module = CameraModule::new("test".to_string());

        // No camera assigned
        assert_eq!(module.status(), ModuleStatus::Error("No camera assigned".to_string()));
    }

    #[tokio::test]
    async fn test_downcast() {
        let mut module: Box<dyn Module> = Box::new(CameraModule::new("test".to_string()));

        // Should be able to downcast to CameraModule
        let camera_module = module.as_any_mut().downcast_mut::<CameraModule>();
        assert!(camera_module.is_some());

        // Can assign camera via downcast
        let mock_camera: Box<dyn Camera> = Box::new(MockCamera {
            id: "mock1".to_string(),
            exposure_ms: 100.0,
        });
        camera_module.unwrap().assign_camera(mock_camera).unwrap();
    }
}
