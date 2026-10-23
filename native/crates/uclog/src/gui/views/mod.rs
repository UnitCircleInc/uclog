//! The dockable views (GUI plan §3).

pub mod cmds;
pub mod device;
pub mod fw;
pub mod logs;
pub mod plots;
pub mod term;

use egui::{Id, Ui, WidgetText};
use egui_dock::TabViewer;

use super::app::{GuiApp, Tab};

pub struct Viewer<'a> {
    pub app: &'a mut GuiApp,
}

impl TabViewer for Viewer<'_> {
    type Tab = Tab;

    fn id(&mut self, tab: &mut Tab) -> Id {
        Id::new(("uclog-tab", &*tab))
    }

    fn title(&mut self, tab: &mut Tab) -> WidgetText {
        tab.title().into()
    }

    fn ui(&mut self, ui: &mut Ui, tab: &mut Tab) {
        match tab {
            Tab::Logs => logs::ui(self.app, ui),
            Tab::Terminal => term::ui(self.app, ui),
            Tab::Commands => cmds::ui(self.app, ui),
            Tab::Device => device::ui(self.app, ui),
            Tab::Fw => fw::ui(self.app, ui),
            Tab::Plot(name) => plots::ui(self.app, ui, name),
        }
    }
}
