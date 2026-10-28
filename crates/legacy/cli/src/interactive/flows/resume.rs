use iocraft::prelude::*;

use crate::interactive::{
    components::{ApplicationState, Selector, SelectorItem, SelectorStyle},
    flows::{Flow, FlowEvent},
};

pub struct ResumeFlow;

impl Flow for ResumeFlow {
    fn render(
        &self,
        on_event: Handler<FlowEvent>,
    ) -> AnyElement<'static> {
        element! { ResumeFlowView(on_event: on_event) }.into()
    }
}

#[derive(Default, Props)]
struct ResumeFlowViewProps {
    pub on_event: Handler<FlowEvent>,
}

#[component]
fn ResumeFlowView(
    props: &mut ResumeFlowViewProps,
    mut hooks: Hooks,
) -> impl Into<AnyElement<'static>> {
    let _on_event = std::mem::take(&mut props.on_event);
    let state = *hooks.use_context::<State<ApplicationState>>();
    let offer = state.read().resume_offer.clone();
    let accent = state.read().theme().accent_color;
    let subtitle = state.read().theme().subtitle_color;
    let padding = state.read().theme().padding();

    let on_pick = hooks.use_async_handler(move |index: usize| {
        let mut state = state;
        async move {
            if index == 0 {
                ApplicationState::continue_last_chat(&mut state).await;
            } else {
                ApplicationState::start_new_chat(&mut state).await;
            }
        }
    });

    let summary = offer.as_ref().map(|offer| offer.summary.clone()).unwrap_or_else(|| "Saved chat".to_string());
    let preview = offer.as_ref().and_then(|offer| offer.preview.clone());
    let preview_line: AnyElement<'static> = match preview {
        Some(preview) => element! {
            Text(content: preview, color: subtitle)
        }
        .into(),
        None => element! { View }.into(),
    };

    element! {
        View(flex_direction: FlexDirection::Column, padding_left: padding, padding_right: padding) {
            Text(content: "Last chat", weight: Weight::Bold, color: accent)
            Text(content: summary, color: subtitle)
            #(preview_line)
            View(height: 1u16)
            Selector(
                items: vec![
                    SelectorItem {
                        title: "Continue".to_string(),
                        description: Some("Pick up where you left off".to_string()),
                        color: None,
                    },
                    SelectorItem {
                        title: "Start new".to_string(),
                        description: Some("Empty chat".to_string()),
                        color: None,
                    },
                ],
                style: SelectorStyle::Plain,
                maximal_height: 2u16,
                accent_color: accent,
                subtitle_color: subtitle,
                columns_padding: state.read().theme().padding_wide(),
                on_submit: move |index: usize| on_pick(index),
            )
        }
    }
}
