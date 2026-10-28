use std::{collections::HashMap, sync::Arc};

use crate::interactive::flows::Flow;

pub type FlowFactory = Arc<dyn Fn() -> Box<dyn Flow> + Send + Sync>;

#[derive(Clone)]
pub enum CommandKind {
    Flow(FlowFactory),
    Compact,
}

#[derive(Clone)]
pub struct Command {
    pub name: String,
    pub description: String,
    pub requires_model: bool,
    pub requires_chat: bool,
    pub kind: CommandKind,
}

#[derive(Clone, Default)]
pub struct FlowRegistry {
    commands: HashMap<String, Command>,
}

impl FlowRegistry {
    pub fn register<F>(
        mut self,
        name: impl Into<String>,
        description: impl Into<String>,
        requires_model: bool,
        factory: F,
    ) -> Self
    where
        F: Fn() -> Box<dyn Flow> + Send + Sync + 'static,
    {
        let name = name.into();
        let command = Command {
            name: name.clone(),
            description: description.into(),
            requires_model,
            requires_chat: false,
            kind: CommandKind::Flow(Arc::new(factory)),
        };
        self.commands.insert(name, command);
        self
    }

    pub fn register_compact(mut self) -> Self {
        self.commands.insert(
            "compact".to_string(),
            Command {
                name: "compact".to_string(),
                description: "Summarize the chat and free context".to_string(),
                requires_model: true,
                requires_chat: true,
                kind: CommandKind::Compact,
            },
        );
        self
    }

    pub fn command(
        &self,
        name: &str,
    ) -> Option<Command> {
        self.commands.get(name).cloned()
    }

    pub fn commands(&self) -> Vec<Command> {
        let mut commands: Vec<Command> = self.commands.values().cloned().collect();
        commands.sort_by(|first, second| first.name.cmp(&second.name));
        commands
    }
}
