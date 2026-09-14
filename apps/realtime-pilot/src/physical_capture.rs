//! Route captures encode PNGs off the main thread and finish before clean exit.
use bevy::{
    prelude::*,
    render::view::screenshot::{Screenshot, ScreenshotCaptured},
    tasks::IoTaskPool,
};
use std::{
    path::PathBuf,
    sync::{Mutex, mpsc},
};

type Receipt = mpsc::Receiver<Result<(), String>>;
#[derive(Resource, Default)]
pub struct CaptureWrites {
    pending: Mutex<Vec<Receipt>>,
    pub finishing: bool,
}
impl CaptureWrites {
    pub fn request(&mut self, commands: &mut Commands, path: PathBuf) {
        // Register before readback, so the final route action also waits for an
        // image whose capture event has not arrived yet. The route has six images.
        let (done, receipt) = mpsc::channel();
        self.pending.get_mut().unwrap().push(receipt);
        commands.spawn(Screenshot::primary_window()).observe(
            move |capture: On<ScreenshotCaptured>| {
                let image = capture.image.clone();
                let path = path.clone();
                let done = done.clone();
                IoTaskPool::get()
                    .spawn(async move {
                        let result = image
                            .try_into_dynamic()
                            .map_err(|e| e.to_string())
                            .and_then(|image| {
                                image.to_rgb8().save(&path).map_err(|e| e.to_string())
                            });
                        let _ = done.send(result.map_err(|e| format!("{}: {e}", path.display())));
                    })
                    .detach();
            },
        );
    }
    pub fn poll(&mut self) -> Result<bool, String> {
        let pending = self.pending.get_mut().unwrap();
        let mut error = None;
        pending.retain(|receipt| match receipt.try_recv() {
            Ok(Ok(())) => false,
            Ok(Err(e)) => {
                error = Some(e);
                false
            }
            Err(mpsc::TryRecvError::Empty) => true,
            Err(mpsc::TryRecvError::Disconnected) => {
                error = Some("screenshot writer disconnected".into());
                false
            }
        });
        match error {
            Some(e) => Err(e),
            None => Ok(pending.is_empty()),
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exit_waits_for_every_capture_and_reports_write_errors() {
        let mut writes = CaptureWrites::default();
        let (first, a) = mpsc::channel();
        let (last, b) = mpsc::channel();
        writes.pending.get_mut().unwrap().extend([a, b]);
        assert!(!writes.poll().unwrap());
        last.send(Ok(())).unwrap();
        assert!(
            !writes.poll().unwrap(),
            "out-of-order completion must retain unfinished images"
        );
        first.send(Ok(())).unwrap();
        assert!(writes.poll().unwrap());
        let (failed, r) = mpsc::channel();
        writes.pending.get_mut().unwrap().push(r);
        failed.send(Err("disk full".into())).unwrap();
        assert_eq!(writes.poll(), Err("disk full".into()));
    }
}
