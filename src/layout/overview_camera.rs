use smithay::utils::{Logical, Point};

use super::workspace::WorkspaceId;
use crate::animation::{Animation, Clock};

#[derive(Debug, Clone, PartialEq)]
pub(super) struct WorkspacePose {
    pub id: WorkspaceId,
    pub location: Point<f64, Logical>,
    pub label_left: f64,
    pub opacity: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct CameraLayout {
    pub zoom: f64,
    pub workspaces: Vec<WorkspacePose>,
    pub open: bool,
}

#[derive(Debug)]
pub(super) struct OverviewCamera {
    from: CameraLayout,
    pub target: CameraLayout,
    animation: Animation,
    settled: bool,
}

impl OverviewCamera {
    pub fn new(
        from: CameraLayout,
        target: CameraLayout,
        clock: Clock,
        config: niri_config::Animation,
    ) -> Self {
        let settled = from == target;
        Self {
            from,
            target,
            animation: Animation::new(clock, 0., 1., 0., config),
            settled,
        }
    }

    pub fn is_animating(&self) -> bool {
        !self.settled && !self.animation.is_done()
    }

    pub fn finish_if_done(&mut self) {
        if self.animation.is_done() {
            self.settled = true;
        }
    }

    fn progress(&self) -> f64 {
        if self.settled {
            1.
        } else {
            self.animation.clamped_value()
        }
    }

    pub fn zoom(&self) -> f64 {
        let p = self.progress();
        self.from.zoom + (self.target.zoom - self.from.zoom) * p
    }

    pub fn pose(&self, id: WorkspaceId) -> Option<WorkspacePose> {
        let to = self.target.workspaces.iter().find(|pose| pose.id == id)?;
        let from = self.from.workspaces.iter().find(|pose| pose.id == id);
        let p = self.progress();
        let (location, label_left, opacity) = match from {
            Some(from) if from.opacity > 0. => (from.location, from.label_left, from.opacity),
            _ => (to.location, to.label_left, 0.),
        };
        Some(WorkspacePose {
            id,
            location: location + (to.location - location).upscale(p),
            label_left: label_left + (to.label_left - label_left) * p,
            opacity: opacity + (to.opacity - opacity) * p,
        })
    }

    pub fn sample(&self) -> CameraLayout {
        CameraLayout {
            zoom: self.zoom(),
            open: self.target.open,
            workspaces: self
                .target
                .workspaces
                .iter()
                .filter_map(|pose| self.pose(pose.id))
                .collect(),
        }
    }
}
