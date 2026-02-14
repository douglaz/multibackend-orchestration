//! PRD user interaction interfaces and implementations.

use std::collections::{BTreeMap, VecDeque};
use std::io::{self, Write};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;

use crate::error::RalphError;
use crate::Result;

use super::gaps::{Question, QuestionKind};
use super::state::Stage;

#[async_trait]
pub trait UserInteraction: Send + Sync {
    async fn ask_questions(
        &self,
        questions: &[Question],
        ctx: &InteractionContext,
    ) -> Result<Option<BTreeMap<String, String>>>;

    fn status(&self, message: &str);

    fn stage_complete(&self, stage: &Stage, summary: &str);

    fn is_interactive(&self) -> bool;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InteractionContext {
    pub stage: Stage,
    pub question_round: u32,
    pub max_rounds: u32,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct PlainInteraction;

impl PlainInteraction {
    pub fn new() -> Self {
        Self
    }

    async fn read_line(&self) -> Result<String> {
        tokio::task::spawn_blocking(|| {
            let mut input = String::new();
            io::stdin().read_line(&mut input)?;
            Ok::<String, io::Error>(input)
        })
        .await
        .map_err(|err| RalphError::PrdPipelineFailed(format!("failed to read from stdin: {err}")))?
        .map_err(Into::into)
    }

    fn print_question(
        &self,
        idx: usize,
        total: usize,
        question: &Question,
        ctx: &InteractionContext,
    ) -> Result<()> {
        println!();
        println!(
            "[{}/{}] {:?} (round {}/{})",
            idx + 1,
            total,
            ctx.stage,
            ctx.question_round,
            ctx.max_rounds
        );
        println!("{}:", question.prompt);

        match &question.kind {
            QuestionKind::FreeText => {
                println!("  type: free text");
            }
            QuestionKind::Choice(options) => {
                println!("  type: choice (enter the option number)");
                for (option_idx, option) in options.iter().enumerate() {
                    println!("    {}. {}", option_idx + 1, option);
                }
            }
            QuestionKind::YesNo => {
                println!("  type: yes/no (y/n)");
            }
        }

        if let Some(default) = &question.suggested_default {
            println!("  default: {default}");
        }

        println!("  commands: :back :edit :show :save :quit");
        print!("> ");
        io::stdout().flush()?;
        Ok(())
    }

    fn show_answers(&self, answers: &BTreeMap<String, String>) {
        if answers.is_empty() {
            println!("No answers collected yet.");
            return;
        }

        println!("Current answers:");
        for (key, value) in answers {
            println!("  {key}: {value}");
        }
    }
}

#[async_trait]
impl UserInteraction for PlainInteraction {
    async fn ask_questions(
        &self,
        questions: &[Question],
        ctx: &InteractionContext,
    ) -> Result<Option<BTreeMap<String, String>>> {
        let mut answers = BTreeMap::new();
        let mut idx = 0_usize;

        while idx < questions.len() {
            let question = &questions[idx];
            self.print_question(idx, questions.len(), question, ctx)?;

            match parse_prompt_input(&self.read_line().await?) {
                PromptInput::Command(command) => match dispatch_command(command, idx) {
                    CommandAction::PreviousQuestion => {
                        idx -= 1;
                        let key = &questions[idx].key;
                        answers.remove(key);
                    }
                    CommandAction::ReaskCurrent => {
                        answers.remove(&question.key);
                    }
                    CommandAction::ShowAnswers => self.show_answers(&answers),
                    CommandAction::SaveAnswers => return Ok(Some(answers)),
                    CommandAction::Quit => return Ok(None),
                    CommandAction::Noop => {
                        self.status("Cannot go back from the first question.");
                    }
                },
                PromptInput::Answer(raw_answer) => {
                    match parse_question_answer(question, &raw_answer) {
                        Ok(answer) => {
                            answers.insert(question.key.clone(), answer);
                            idx += 1;
                        }
                        Err(err) => self.status(&err),
                    }
                }
            }
        }

        Ok(Some(answers))
    }

    fn status(&self, message: &str) {
        println!("{message}");
    }

    fn stage_complete(&self, stage: &Stage, summary: &str) {
        println!("Completed {:?}: {}", stage, summary);
    }

    fn is_interactive(&self) -> bool {
        true
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct NonInteractiveInteraction;

impl NonInteractiveInteraction {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl UserInteraction for NonInteractiveInteraction {
    async fn ask_questions(
        &self,
        _questions: &[Question],
        _ctx: &InteractionContext,
    ) -> Result<Option<BTreeMap<String, String>>> {
        Ok(None)
    }

    fn status(&self, _message: &str) {}

    fn stage_complete(&self, _stage: &Stage, _summary: &str) {}

    fn is_interactive(&self) -> bool {
        false
    }
}

#[derive(Debug, Clone, Default)]
pub struct MockInteraction {
    queued_answers: Arc<Mutex<QueuedAnswers>>,
    status_messages: Arc<Mutex<Vec<String>>>,
    stage_completions: Arc<Mutex<Vec<(Stage, String)>>>,
}

type QueuedAnswers = VecDeque<Option<BTreeMap<String, String>>>;

impl MockInteraction {
    pub fn new(canned_answers: Vec<Option<BTreeMap<String, String>>>) -> Self {
        Self {
            queued_answers: Arc::new(Mutex::new(canned_answers.into())),
            status_messages: Arc::new(Mutex::new(Vec::new())),
            stage_completions: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub fn status_messages(&self) -> Vec<String> {
        self.status_messages
            .lock()
            .expect("mock status lock poisoned")
            .clone()
    }

    pub fn stage_completions(&self) -> Vec<(Stage, String)> {
        self.stage_completions
            .lock()
            .expect("mock stage completion lock poisoned")
            .clone()
    }
}

#[async_trait]
impl UserInteraction for MockInteraction {
    async fn ask_questions(
        &self,
        _questions: &[Question],
        _ctx: &InteractionContext,
    ) -> Result<Option<BTreeMap<String, String>>> {
        let mut answers = self
            .queued_answers
            .lock()
            .expect("mock queued answers lock poisoned");
        Ok(answers.pop_front().unwrap_or(None))
    }

    fn status(&self, message: &str) {
        self.status_messages
            .lock()
            .expect("mock status lock poisoned")
            .push(message.to_owned());
    }

    fn stage_complete(&self, stage: &Stage, summary: &str) {
        self.stage_completions
            .lock()
            .expect("mock stage completion lock poisoned")
            .push((*stage, summary.to_owned()));
    }

    fn is_interactive(&self) -> bool {
        true
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InteractionCommand {
    Back,
    Edit,
    Show,
    Save,
    Quit,
}

impl InteractionCommand {
    fn parse(input: &str) -> Option<Self> {
        match input.trim().to_ascii_lowercase().as_str() {
            ":back" => Some(Self::Back),
            ":edit" => Some(Self::Edit),
            ":show" => Some(Self::Show),
            ":save" => Some(Self::Save),
            ":quit" => Some(Self::Quit),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum PromptInput {
    Command(InteractionCommand),
    Answer(String),
}

fn parse_prompt_input(input: &str) -> PromptInput {
    let trimmed = input.trim();
    if let Some(command) = InteractionCommand::parse(trimmed) {
        PromptInput::Command(command)
    } else {
        PromptInput::Answer(trimmed.to_owned())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CommandAction {
    PreviousQuestion,
    ReaskCurrent,
    ShowAnswers,
    SaveAnswers,
    Quit,
    Noop,
}

fn dispatch_command(command: InteractionCommand, current_idx: usize) -> CommandAction {
    match command {
        InteractionCommand::Back => {
            if current_idx == 0 {
                CommandAction::Noop
            } else {
                CommandAction::PreviousQuestion
            }
        }
        InteractionCommand::Edit => CommandAction::ReaskCurrent,
        InteractionCommand::Show => CommandAction::ShowAnswers,
        InteractionCommand::Save => CommandAction::SaveAnswers,
        InteractionCommand::Quit => CommandAction::Quit,
    }
}

fn parse_question_answer(
    question: &Question,
    raw_input: &str,
) -> std::result::Result<String, String> {
    let trimmed = raw_input.trim();
    let effective = if trimmed.is_empty() {
        question
            .suggested_default
            .as_deref()
            .unwrap_or("")
            .trim()
            .to_owned()
    } else {
        trimmed.to_owned()
    };

    if effective.is_empty() {
        return Err("Answer cannot be empty.".to_owned());
    }

    match &question.kind {
        QuestionKind::FreeText => Ok(effective),
        QuestionKind::Choice(options) => parse_choice_answer(options, &effective),
        QuestionKind::YesNo => parse_yes_no_answer(&effective),
    }
}

fn parse_choice_answer(options: &[String], value: &str) -> std::result::Result<String, String> {
    if options.is_empty() {
        return Err("Choice question has no options.".to_owned());
    }

    if let Ok(index) = value.parse::<usize>() {
        if (1..=options.len()).contains(&index) {
            return Ok(options[index - 1].clone());
        }
        return Err(format!(
            "Please enter a number between 1 and {}.",
            options.len()
        ));
    }

    if let Some(option) = options
        .iter()
        .find(|option| option.eq_ignore_ascii_case(value))
    {
        return Ok(option.clone());
    }

    Err(format!(
        "Invalid choice. Enter a number between 1 and {}.",
        options.len()
    ))
}

fn parse_yes_no_answer(value: &str) -> std::result::Result<String, String> {
    match value.trim().to_ascii_lowercase().as_str() {
        "y" | "yes" => Ok("yes".to_owned()),
        "n" | "no" => Ok("no".to_owned()),
        _ => Err("Please answer with 'y' or 'n'.".to_owned()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn context() -> InteractionContext {
        InteractionContext {
            stage: Stage::Research,
            question_round: 1,
            max_rounds: 3,
        }
    }

    fn sample_question(kind: QuestionKind) -> Question {
        Question {
            key: "q_platform".to_owned(),
            prompt: "Which platform?".to_owned(),
            kind,
            suggested_default: None,
            impact_stage: Stage::Research,
        }
    }

    #[tokio::test]
    async fn mock_interaction_returns_canned_answers_and_none_when_exhausted() {
        let mut first = BTreeMap::new();
        first.insert("k1".to_owned(), "v1".to_owned());
        let mut second = BTreeMap::new();
        second.insert("k2".to_owned(), "v2".to_owned());

        let interaction =
            MockInteraction::new(vec![Some(first.clone()), None, Some(second.clone())]);

        assert_eq!(
            interaction.ask_questions(&[], &context()).await.unwrap(),
            Some(first)
        );
        assert_eq!(
            interaction.ask_questions(&[], &context()).await.unwrap(),
            None
        );
        assert_eq!(
            interaction.ask_questions(&[], &context()).await.unwrap(),
            Some(second)
        );
        assert_eq!(
            interaction.ask_questions(&[], &context()).await.unwrap(),
            None
        );

        interaction.status("status message");
        interaction.stage_complete(&Stage::Synthesis, "summary");

        assert_eq!(
            interaction.status_messages(),
            vec!["status message".to_owned()]
        );
        assert_eq!(
            interaction.stage_completions(),
            vec![(Stage::Synthesis, "summary".to_owned())]
        );
    }

    #[tokio::test]
    async fn non_interactive_interaction_always_returns_none() {
        let interaction = NonInteractiveInteraction::new();
        let question = sample_question(QuestionKind::FreeText);

        assert_eq!(
            interaction
                .ask_questions(&[question], &context())
                .await
                .unwrap(),
            None
        );
        assert_eq!(
            interaction.ask_questions(&[], &context()).await.unwrap(),
            None
        );
    }

    #[test]
    fn plain_interaction_parses_commands() {
        assert_eq!(
            parse_prompt_input(":back"),
            PromptInput::Command(InteractionCommand::Back)
        );
        assert_eq!(
            parse_prompt_input(":edit"),
            PromptInput::Command(InteractionCommand::Edit)
        );
        assert_eq!(
            parse_prompt_input(":show"),
            PromptInput::Command(InteractionCommand::Show)
        );
        assert_eq!(
            parse_prompt_input(":save"),
            PromptInput::Command(InteractionCommand::Save)
        );
        assert_eq!(
            parse_prompt_input(":quit"),
            PromptInput::Command(InteractionCommand::Quit)
        );

        assert_eq!(
            parse_prompt_input("some answer"),
            PromptInput::Answer("some answer".to_owned())
        );
    }

    #[test]
    fn plain_interaction_dispatches_commands() {
        assert_eq!(
            dispatch_command(InteractionCommand::Back, 0),
            CommandAction::Noop
        );
        assert_eq!(
            dispatch_command(InteractionCommand::Back, 2),
            CommandAction::PreviousQuestion
        );
        assert_eq!(
            dispatch_command(InteractionCommand::Edit, 1),
            CommandAction::ReaskCurrent
        );
        assert_eq!(
            dispatch_command(InteractionCommand::Show, 1),
            CommandAction::ShowAnswers
        );
        assert_eq!(
            dispatch_command(InteractionCommand::Save, 1),
            CommandAction::SaveAnswers
        );
        assert_eq!(
            dispatch_command(InteractionCommand::Quit, 1),
            CommandAction::Quit
        );
    }

    #[test]
    fn parse_question_answer_handles_defaults_and_kinds() {
        let mut free_text = sample_question(QuestionKind::FreeText);
        free_text.suggested_default = Some("default text".to_owned());
        assert_eq!(
            parse_question_answer(&free_text, "").unwrap(),
            "default text".to_owned()
        );

        let choice = sample_question(QuestionKind::Choice(vec![
            "Web".to_owned(),
            "Mobile".to_owned(),
        ]));
        assert_eq!(
            parse_question_answer(&choice, "2").unwrap(),
            "Mobile".to_owned()
        );
        assert!(parse_question_answer(&choice, "3").is_err());

        let yes_no = sample_question(QuestionKind::YesNo);
        assert_eq!(
            parse_question_answer(&yes_no, "y").unwrap(),
            "yes".to_owned()
        );
        assert_eq!(
            parse_question_answer(&yes_no, "N").unwrap(),
            "no".to_owned()
        );
        assert!(parse_question_answer(&yes_no, "maybe").is_err());
    }
}
