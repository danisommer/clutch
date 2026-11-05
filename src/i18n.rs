//! Interface languages: every text shown to the user, in Brazilian Portuguese and English.

use serde::{Deserialize, Serialize};

use crate::admin::AdminError;
use crate::cleanup::Group;
use crate::storage::{HistoryDate, HistoryKind, LanguageChoice, Stamp, Summary, format_duration};
use crate::system::{self, PowerMode, ServiceError};

const LANG_PORTUGUESE: u16 = 0x16;
const PRIMARY_LANGUAGE_MASK: u16 = 0x3FF;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Language {
    PtBr,
    En,
}

/// The language to show: the chosen one, or Portuguese when Windows is in Portuguese and English otherwise.
pub fn resolve(choice: LanguageChoice) -> Language {
    match choice {
        LanguageChoice::System => match system::user_ui_language() & PRIMARY_LANGUAGE_MASK {
            LANG_PORTUGUESE => Language::PtBr,
            _ => Language::En,
        },
        LanguageChoice::PortugueseBrazil => Language::PtBr,
        LanguageChoice::English => Language::En,
    }
}

pub fn texts(language: Language) -> &'static Texts {
    match language {
        Language::PtBr => &PT_BR,
        Language::En => &EN,
    }
}

pub fn texts_for(choice: LanguageChoice) -> &'static Texts {
    texts(resolve(choice))
}

/// Catalog text: one string per language, or a single string used in every language.
#[derive(Clone, Debug, Deserialize)]
#[serde(untagged)]
pub enum LocalizedText {
    Translated {
        #[serde(rename = "pt-BR")]
        pt_br: String,
        en: String,
    },
    Same(String),
}

impl LocalizedText {
    pub fn get(&self, language: Language) -> &str {
        match (self, language) {
            (LocalizedText::Translated { pt_br, .. }, Language::PtBr) => pt_br,
            (LocalizedText::Translated { en, .. }, Language::En) => en,
            (LocalizedText::Same(text), _) => text,
        }
    }

    pub fn is_translated(&self) -> bool {
        matches!(self, LocalizedText::Translated { .. })
    }
}

/// Fixed texts of one language. Texts with numbers or names are the methods below.
pub struct Texts {
    pub language: Language,

    pub game_mode: &'static str,
    pub cleanup: &'static str,
    pub history: &'static str,
    pub settings: &'static str,
    pub drive_c: &'static str,

    pub game_mode_subtitle: &'static str,
    pub preparing_subtitle: &'static str,
    pub restoring_subtitle: &'static str,
    pub reopening_programs: &'static str,
    pub status_section: &'static str,
    pub refresh: &'static str,
    pub checking: &'static str,
    pub close_programs_section: &'static str,
    pub remove: &'static str,
    pub add_open_program: &'static str,
    pub no_other_windows: &'static str,
    pub choose_program: &'static str,
    pub system_section: &'static str,
    pub shutdown_wsl: &'static str,
    pub best_performance: &'static str,
    pub reopen_on_exit: &'static str,
    pub launch_game_section: &'static str,
    pub no_game: &'static str,
    pub activate: &'static str,
    pub active_tag: &'static str,
    pub no_game_chosen: &'static str,
    pub keep_open: &'static str,
    pub what_was_done: &'static str,
    pub session_resumed: &'static str,
    pub deactivate: &'static str,
    pub finished_subtitle: &'static str,
    pub on_exit_section: &'static str,
    pub nothing_restored: &'static str,
    pub back: &'static str,
    pub step_preparing: &'static str,
    pub step_closing: &'static str,
    pub step_wsl: &'static str,
    pub step_power: &'static str,
    pub step_game: &'static str,

    pub closed: &'static str,
    pub could_not_close: &'static str,
    pub close_manually: &'static str,
    pub still_open: &'static str,
    pub maybe_unsaved: &'static str,
    pub wsl_running: &'static str,
    pub wsl_off: &'static str,
    pub wsl_kept_running: &'static str,
    pub wsl_did_not_stop: &'static str,
    pub wsl_shutdown_hint: &'static str,
    pub charger: &'static str,
    pub plugged_in: &'static str,
    pub unplugged: &'static str,
    pub plug_in: &'static str,
    pub power_mode: &'static str,
    pub limited_by_saver: &'static str,
    pub turn_off_saver: &'static str,
    pub could_not_set_best: &'static str,
    pub unknown: &'static str,
    pub could_not_restore: &'static str,
    pub power_settings_hint: &'static str,
    pub lenovo_mode: &'static str,
    pub lenovo_adaptive: &'static str,
    pub lenovo_performance: &'static str,
    pub lenovo_battery_saving: &'static str,
    pub lenovo_check: &'static str,
    pub lenovo_select_performance: &'static str,
    pub lenovo_back_to_adaptive: &'static str,
    pub available_memory: &'static str,
    pub low_memory: &'static str,
    pub laptop_screen: &'static str,
    pub external_monitor: &'static str,
    pub primary_screen: &'static str,
    pub second_screen_only: &'static str,
    pub open: &'static str,
    pub close_if_not_monitoring: &'static str,
    pub heavy_programs: &'static str,
    pub none_open: &'static str,
    pub game: &'static str,
    pub could_not_launch_steam: &'static str,
    pub reopened: &'static str,
    pub not_reopened: &'static str,
    pub open_manually: &'static str,

    pub cleanup_subtitle: &'static str,
    pub cleanup_intro: &'static str,
    pub analyze: &'static str,
    pub waiting_for_admin: &'static str,
    pub found: &'static str,
    pub analyze_again: &'static str,
    pub measure_system: &'static str,
    pub asks_for_admin: &'static str,
    pub tag_admin: &'static str,
    pub tag_slow: &'static str,
    pub tag_caution: &'static str,
    pub close_now: &'static str,
    pub selected: &'static str,
    pub plus_system: &'static str,
    pub clean_selected: &'static str,
    pub freed: &'static str,
    pub nothing_removed: &'static str,
    pub system_items_waiting: &'static str,
    pub open_programs: &'static str,
    pub permission_denied_measure: &'static str,
    pub permission_denied_clean: &'static str,
    pub admin_no_result: &'static str,

    pub history_subtitle: &'static str,
    pub history_empty: &'static str,

    pub settings_subtitle: &'static str,
    pub theme_section: &'static str,
    pub follow_windows: &'static str,
    pub theme_light: &'static str,
    pub theme_dark: &'static str,
    pub language_section: &'static str,
}

impl Texts {
    pub fn free_space(&self, size: &str) -> String {
        match self.language {
            Language::PtBr => format!("{size} livres"),
            Language::En => format!("{size} free"),
        }
    }

    pub fn game_running(&self, game: &str) -> String {
        match self.language {
            Language::PtBr => format!("{game} em execução. O Modo Jogo encerra sozinho quando o jogo fechar."),
            Language::En => format!("{game} is running. Game Mode ends by itself when the game closes."),
        }
    }

    pub fn waiting_for_game(&self, game: &str) -> String {
        match self.language {
            Language::PtBr => format!("Aguardando {game} abrir."),
            Language::En => format!("Waiting for {game} to start."),
        }
    }

    pub fn window_closes_in(&self, seconds: u64) -> String {
        match self.language {
            Language::PtBr => format!("Esta janela fecha em {seconds} s para liberar memória. O Modo Jogo continua ativo."),
            Language::En => format!("This window closes in {seconds} s to free memory. Game Mode stays on."),
        }
    }

    pub fn ran_in_background(&self, program: &str) -> String {
        match self.language {
            Language::PtBr => format!("rodava em segundo plano; desative isso nas configurações do {program}"),
            Language::En => format!("it was running in the background; turn that off in the {program} settings"),
        }
    }

    pub fn wsl_in_use_by(&self, programs: &str) -> String {
        match self.language {
            Language::PtBr => format!("em uso por {programs}"),
            Language::En => format!("in use by {programs}"),
        }
    }

    pub fn power_mode_name(&self, mode: PowerMode) -> &'static str {
        match (self.language, mode) {
            (Language::PtBr, PowerMode::BestPerformance) => "Melhor desempenho",
            (Language::PtBr, PowerMode::Balanced) => "Equilibrado",
            (Language::PtBr, PowerMode::BestEfficiency) => "Melhor eficiência de energia",
            (Language::PtBr, PowerMode::Unknown) => "desconhecido",
            (Language::En, PowerMode::BestPerformance) => "Best performance",
            (Language::En, PowerMode::Balanced) => "Balanced",
            (Language::En, PowerMode::BestEfficiency) => "Best power efficiency",
            (Language::En, PowerMode::Unknown) => "unknown",
        }
    }

    pub fn power_back_to(&self, mode: &str) -> String {
        match self.language {
            Language::PtBr => format!("voltou para {mode}"),
            Language::En => format!("back to {mode}"),
        }
    }

    pub fn screen_mode(&self, width: u32, height: u32, hz: u32, primary: bool) -> String {
        match primary {
            true => format!("{width}x{height}, {hz} Hz · {}", self.primary_screen),
            false => format!("{width}x{height}, {hz} Hz"),
        }
    }

    pub fn supports_hz(&self, hz: u32) -> String {
        match self.language {
            Language::PtBr => format!("suporta {hz} Hz: Configurações > Sistema > Tela > Tela avançada"),
            Language::En => format!("supports {hz} Hz: Settings > System > Display > Advanced display"),
        }
    }

    pub fn analyzing(&self, item: &str) -> String {
        match self.language {
            Language::PtBr => format!("Analisando {item}…"),
            Language::En => format!("Analyzing {item}…"),
        }
    }

    pub fn cleaning(&self, item: &str) -> String {
        match self.language {
            Language::PtBr => format!("Limpando {item}…"),
            Language::En => format!("Cleaning {item}…"),
        }
    }

    pub fn group(&self, group: Group) -> &'static str {
        match (self.language, group) {
            (Language::PtBr, Group::System) => "Sistema",
            (Language::PtBr, Group::Programs) => "Programas",
            (Language::PtBr, Group::Leftovers) => "Restos",
            (Language::PtBr, Group::Caution) => "Atenção",
            (Language::En, Group::System) => "System",
            (Language::En, Group::Programs) => "Programs",
            (Language::En, Group::Leftovers) => "Leftovers",
            (Language::En, Group::Caution) => "Caution",
        }
    }

    pub fn close_to_clean(&self, programs: &str) -> String {
        match self.language {
            Language::PtBr => format!("Feche {programs} para limpar."),
            Language::En => format!("Close {programs} to clean."),
        }
    }

    pub fn close_first(&self, programs: &str) -> String {
        match self.language {
            Language::PtBr => format!("feche: {programs}"),
            Language::En => format!("close: {programs}"),
        }
    }

    pub fn kept_in_use(&self, count: u64) -> String {
        match (self.language, count) {
            (Language::PtBr, 1) => "1 item em uso foi mantido".to_string(),
            (Language::PtBr, n) => format!("{n} itens em uso foram mantidos"),
            (Language::En, 1) => "1 item in use was kept".to_string(),
            (Language::En, n) => format!("{n} items in use were kept"),
        }
    }

    pub fn sessions(&self, count: usize) -> String {
        match (self.language, count) {
            (Language::PtBr, 1) => "1 sessão".to_string(),
            (Language::PtBr, n) => format!("{n} sessões"),
            (Language::En, 1) => "1 session".to_string(),
            (Language::En, n) => format!("{n} sessions"),
        }
    }

    pub fn stale_extensions(&self, count: usize) -> String {
        match (self.language, count) {
            (Language::PtBr, 1) => "1 extensão desatualizada".to_string(),
            (Language::PtBr, n) => format!("{n} extensões desatualizadas"),
            (Language::En, 1) => "1 outdated extension".to_string(),
            (Language::En, n) => format!("{n} outdated extensions"),
        }
    }

    pub fn categories(&self, count: usize) -> String {
        match (self.language, count) {
            (Language::PtBr, 1) => "1 categoria".to_string(),
            (Language::PtBr, n) => format!("{n} categorias"),
            (Language::En, 1) => "1 category".to_string(),
            (Language::En, n) => format!("{n} categories"),
        }
    }

    pub fn could_not_clean(&self, error: &str) -> String {
        match self.language {
            Language::PtBr => format!("não foi possível limpar: {error}"),
            Language::En => format!("could not clean: {error}"),
        }
    }

    pub fn could_not_run_dism(&self, error: &str) -> String {
        match self.language {
            Language::PtBr => format!("não foi possível rodar o DISM: {error}"),
            Language::En => format!("could not run DISM: {error}"),
        }
    }

    pub fn dism_exit_code(&self, code: i32) -> String {
        match self.language {
            Language::PtBr => format!("o DISM terminou com código {code}"),
            Language::En => format!("DISM ended with code {code}"),
        }
    }

    pub fn service_error(&self, service: &str, error: ServiceError) -> String {
        match (self.language, error) {
            (Language::PtBr, ServiceError::NoManager(code)) => format!("sem acesso ao gerenciador de serviços ({code})"),
            (Language::PtBr, ServiceError::Unavailable(code)) => format!("serviço {service} indisponível ({code})"),
            (Language::En, ServiceError::NoManager(code)) => format!("no access to the service manager ({code})"),
            (Language::En, ServiceError::Unavailable(code)) => format!("service {service} unavailable ({code})"),
        }
    }

    /// Message for a failed elevated run; `cancelled` is the text for when the user refused the prompt.
    pub fn admin_error(&self, error: AdminError, cancelled: &str) -> String {
        match (self.language, error) {
            (_, AdminError::Cancelled) => cancelled.to_string(),
            (_, AdminError::NoResult) => self.admin_no_result.to_string(),
            (_, AdminError::Io(message)) => message,
            (Language::PtBr, AdminError::RequestFailed(code)) => format!("não foi possível pedir permissão ({code})"),
            (Language::PtBr, AdminError::ExitCode(code)) => format!("o modo administrador terminou com código {code}"),
            (Language::En, AdminError::RequestFailed(code)) => format!("could not ask for permission ({code})"),
            (Language::En, AdminError::ExitCode(code)) => format!("the administrator mode ended with code {code}"),
        }
    }

    pub fn history_kind(&self, kind: HistoryKind) -> &'static str {
        match kind {
            HistoryKind::GameMode => self.game_mode,
            HistoryKind::Cleanup => self.cleanup,
        }
    }

    pub fn date(&self, stamp: &Stamp) -> String {
        let Stamp { year, month, day, hour, minute } = *stamp;
        match self.language {
            Language::PtBr => format!("{day:02}/{month:02}/{year} {hour:02}:{minute:02}"),
            Language::En => format!("{year}-{month:02}-{day:02} {hour:02}:{minute:02}"),
        }
    }

    pub fn history_date(&self, date: &HistoryDate) -> String {
        match date {
            HistoryDate::Local(stamp) => self.date(stamp),
            HistoryDate::Legacy(text) => text.clone(),
        }
    }

    pub fn history_summary(&self, summary: &Summary) -> String {
        match (self.language, summary) {
            (_, Summary::Cleanup { categories }) => self.categories(*categories),
            (_, Summary::Legacy(text)) => text.clone(),
            (Language::PtBr, Summary::GameMode { game: Some(game), seconds }) => format!("{game} por {}", format_duration(*seconds)),
            (Language::PtBr, Summary::GameMode { game: None, seconds }) => format!("Modo Jogo por {}", format_duration(*seconds)),
            (Language::En, Summary::GameMode { game: Some(game), seconds }) => format!("{game} for {}", format_duration(*seconds)),
            (Language::En, Summary::GameMode { game: None, seconds }) => format!("Game Mode for {}", format_duration(*seconds)),
        }
    }

    /// Human-readable size: "6,4 GB" in Portuguese, "6.4 GB" in English.
    pub fn size(&self, bytes: u64) -> String {
        const KB: f64 = 1024.0;
        let b = bytes as f64;
        let (value, unit, decimals) = match b {
            _ if b >= KB * KB * KB => (b / (KB * KB * KB), "GB", 1),
            _ if b >= KB * KB => (b / (KB * KB), "MB", 0),
            _ if b >= KB => (b / KB, "KB", 0),
            _ => (b, "B", 0),
        };
        let formatted = format!("{value:.decimals$} {unit}");
        match self.language {
            Language::PtBr => formatted.replace('.', ","),
            Language::En => formatted,
        }
    }
}

pub static PT_BR: Texts = Texts {
    language: Language::PtBr,

    game_mode: "Modo Jogo",
    cleanup: "Limpeza",
    history: "Histórico",
    settings: "Configurações",
    drive_c: "DISCO C:",

    game_mode_subtitle: "Fecha programas pesados, desliga a WSL e ajusta a energia para jogar.",
    preparing_subtitle: "Preparando o computador.",
    restoring_subtitle: "Restaurando o computador.",
    reopening_programs: "Reabrindo programas…",
    status_section: "Estado",
    refresh: "Atualizar",
    checking: "Verificando…",
    close_programs_section: "Fechar programas",
    remove: "Remover",
    add_open_program: "+ Adicionar programa aberto",
    no_other_windows: "Nenhum outro programa com janela aberta.",
    choose_program: "Escolha o programa:",
    system_section: "Sistema",
    shutdown_wsl: "Desligar a WSL",
    best_performance: "Ativar Melhor desempenho",
    reopen_on_exit: "Reabrir programas ao sair do jogo",
    launch_game_section: "Abrir jogo",
    no_game: "Nenhum",
    activate: "Ativar Modo Jogo",
    active_tag: "Ativo",
    no_game_chosen: "Nenhum jogo escolhido. Encerre quando terminar de jogar.",
    keep_open: "Manter aberta",
    what_was_done: "O que foi feito",
    session_resumed: "Sessão retomada depois que o app foi reaberto.",
    deactivate: "Encerrar Modo Jogo",
    finished_subtitle: "Encerrado. O computador voltou ao normal.",
    on_exit_section: "Ao encerrar",
    nothing_restored: "Nada precisou ser restaurado.",
    back: "Voltar",
    step_preparing: "Preparando",
    step_closing: "Fechando programas",
    step_wsl: "Desligando a WSL",
    step_power: "Ajustando energia",
    step_game: "Abrindo o jogo",

    closed: "fechado",
    could_not_close: "não foi possível fechar",
    close_manually: "feche manualmente",
    still_open: "continua aberto",
    maybe_unsaved: "pode haver algo não salvo; feche manualmente",
    wsl_running: "ligada",
    wsl_off: "desligada",
    wsl_kept_running: "mantida ligada",
    wsl_did_not_stop: "não desligou",
    wsl_shutdown_hint: "rode \"wsl --shutdown\" em um terminal",
    charger: "Carregador",
    plugged_in: "conectado",
    unplugged: "desconectado",
    plug_in: "conecte o carregador",
    power_mode: "Modo de energia",
    limited_by_saver: "limitado pela Economia de energia",
    turn_off_saver: "desative a Economia de energia (Win+A)",
    could_not_set_best: "não foi possível ativar Melhor desempenho",
    unknown: "não identificado",
    could_not_restore: "não foi possível restaurar",
    power_settings_hint: "ajuste em Configurações > Sistema > Energia",
    lenovo_mode: "Modo Lenovo",
    lenovo_adaptive: "Energia adaptável",
    lenovo_performance: "Desempenho",
    lenovo_battery_saving: "Economia de bateria",
    lenovo_check: "confira no Lenovo Vantage",
    lenovo_select_performance: "selecione Desempenho no Lenovo Vantage",
    lenovo_back_to_adaptive: "volte para Energia adaptável no Lenovo Vantage",
    available_memory: "Memória disponível",
    low_memory: "pouca memória; feche outros programas",
    laptop_screen: "Tela do notebook",
    external_monitor: "Monitor externo",
    primary_screen: "principal",
    second_screen_only: "para jogar só nele: Win+P > Somente segunda tela",
    open: "aberto",
    close_if_not_monitoring: "feche se não estiver monitorando",
    heavy_programs: "Programas pesados",
    none_open: "nenhum aberto",
    game: "Jogo",
    could_not_launch_steam: "não foi possível abrir pela Steam",
    reopened: "reaberto",
    not_reopened: "não foi reaberto",
    open_manually: "abra manualmente",

    cleanup_subtitle: "Analise o que pode ser removido e marque o que limpar. Nada é apagado sem você confirmar.",
    cleanup_intro: "Arquivos temporários, caches de programas e restos de instalações.",
    analyze: "Analisar",
    waiting_for_admin: "Aguardando a permissão de administrador…",
    found: "Encontrado",
    analyze_again: "Analisar de novo",
    measure_system: "Medir itens do sistema",
    asks_for_admin: "Pede permissão de administrador",
    tag_admin: "Admin",
    tag_slow: "Demorado",
    tag_caution: "Atenção",
    close_now: "Fechar agora",
    selected: "Selecionado",
    plus_system: " + sistema",
    clean_selected: "Limpar selecionados",
    freed: "Liberado",
    nothing_removed: "nada removido",
    system_items_waiting: "itens do sistema (aguardando permissão)",
    open_programs: "programas abertos",
    permission_denied_measure: "Permissão negada: os itens do sistema não foram medidos.",
    permission_denied_clean: "Permissão negada: os itens do sistema não foram limpos.",
    admin_no_result: "o modo administrador não devolveu resultado",

    history_subtitle: "Limpezas feitas e sessões de Modo Jogo.",
    history_empty: "Nada por aqui ainda.",

    settings_subtitle: "Aparência e preferências do app.",
    theme_section: "Tema",
    follow_windows: "Seguir o Windows",
    theme_light: "Claro",
    theme_dark: "Escuro",
    language_section: "Idioma",
};

pub static EN: Texts = Texts {
    language: Language::En,

    game_mode: "Game Mode",
    cleanup: "Cleanup",
    history: "History",
    settings: "Settings",
    drive_c: "DRIVE C:",

    game_mode_subtitle: "Closes heavy programs, shuts down WSL and sets the power mode for gaming.",
    preparing_subtitle: "Getting the computer ready.",
    restoring_subtitle: "Restoring the computer.",
    reopening_programs: "Reopening programs…",
    status_section: "Status",
    refresh: "Refresh",
    checking: "Checking…",
    close_programs_section: "Close programs",
    remove: "Remove",
    add_open_program: "+ Add an open program",
    no_other_windows: "No other program has an open window.",
    choose_program: "Choose the program:",
    system_section: "System",
    shutdown_wsl: "Shut down WSL",
    best_performance: "Turn on Best performance",
    reopen_on_exit: "Reopen programs after the game",
    launch_game_section: "Launch game",
    no_game: "None",
    activate: "Turn on Game Mode",
    active_tag: "On",
    no_game_chosen: "No game chosen. End Game Mode when you finish playing.",
    keep_open: "Keep it open",
    what_was_done: "What was done",
    session_resumed: "Session resumed after the app was reopened.",
    deactivate: "End Game Mode",
    finished_subtitle: "Ended. The computer is back to normal.",
    on_exit_section: "On exit",
    nothing_restored: "Nothing needed to be restored.",
    back: "Back",
    step_preparing: "Preparing",
    step_closing: "Closing programs",
    step_wsl: "Shutting down WSL",
    step_power: "Setting the power mode",
    step_game: "Launching the game",

    closed: "closed",
    could_not_close: "could not be closed",
    close_manually: "close it manually",
    still_open: "still open",
    maybe_unsaved: "something may be unsaved; close it manually",
    wsl_running: "running",
    wsl_off: "off",
    wsl_kept_running: "kept running",
    wsl_did_not_stop: "did not shut down",
    wsl_shutdown_hint: "run \"wsl --shutdown\" in a terminal",
    charger: "Charger",
    plugged_in: "plugged in",
    unplugged: "unplugged",
    plug_in: "plug in the charger",
    power_mode: "Power mode",
    limited_by_saver: "limited by Energy saver",
    turn_off_saver: "turn off Energy saver (Win+A)",
    could_not_set_best: "could not turn on Best performance",
    unknown: "unknown",
    could_not_restore: "could not be restored",
    power_settings_hint: "set it in Settings > System > Power & battery",
    lenovo_mode: "Lenovo mode",
    lenovo_adaptive: "Adaptive",
    lenovo_performance: "Performance",
    lenovo_battery_saving: "Battery saving",
    lenovo_check: "check it in Lenovo Vantage",
    lenovo_select_performance: "select Performance in Lenovo Vantage",
    lenovo_back_to_adaptive: "switch back to Adaptive in Lenovo Vantage",
    available_memory: "Available memory",
    low_memory: "low memory; close other programs",
    laptop_screen: "Laptop screen",
    external_monitor: "External monitor",
    primary_screen: "primary",
    second_screen_only: "to play only on it: Win+P > Second screen only",
    open: "open",
    close_if_not_monitoring: "close it if you are not monitoring",
    heavy_programs: "Heavy programs",
    none_open: "none open",
    game: "Game",
    could_not_launch_steam: "could not be launched through Steam",
    reopened: "reopened",
    not_reopened: "not reopened",
    open_manually: "open it manually",

    cleanup_subtitle: "Analyze what can be removed and check what to clean. Nothing is deleted until you confirm.",
    cleanup_intro: "Temporary files, program caches and installation leftovers.",
    analyze: "Analyze",
    waiting_for_admin: "Waiting for administrator permission…",
    found: "Found",
    analyze_again: "Analyze again",
    measure_system: "Measure system items",
    asks_for_admin: "Asks for administrator permission",
    tag_admin: "Admin",
    tag_slow: "Slow",
    tag_caution: "Caution",
    close_now: "Close now",
    selected: "Selected",
    plus_system: " + system",
    clean_selected: "Clean selected",
    freed: "Freed",
    nothing_removed: "nothing removed",
    system_items_waiting: "system items (waiting for permission)",
    open_programs: "open programs",
    permission_denied_measure: "Permission denied: the system items were not measured.",
    permission_denied_clean: "Permission denied: the system items were not cleaned.",
    admin_no_result: "the administrator mode returned no result",

    history_subtitle: "Cleanups done and Game Mode sessions.",
    history_empty: "Nothing here yet.",

    settings_subtitle: "Appearance and app preferences.",
    theme_section: "Theme",
    follow_windows: "Follow Windows",
    theme_light: "Light",
    theme_dark: "Dark",
    language_section: "Language",
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_sizes_per_language() {
        assert_eq!(PT_BR.size(0), "0 B");
        assert_eq!(PT_BR.size(1536), "2 KB");
        assert_eq!(PT_BR.size(820 * 1024 * 1024), "820 MB");
        assert_eq!(PT_BR.size(6_871_947_674), "6,4 GB");
        assert_eq!(EN.size(6_871_947_674), "6.4 GB");
    }

    #[test]
    fn formats_dates_per_language() {
        let stamp = Stamp { year: 2026, month: 11, day: 4, hour: 9, minute: 5 };
        assert_eq!(PT_BR.date(&stamp), "04/11/2026 09:05");
        assert_eq!(EN.date(&stamp), "2026-11-04 09:05");
    }

    #[test]
    fn reads_old_history_entries() {
        let old = r#"[{"date":"04/11/2026 16:00","kind":"Cleanup","freed":1024,"summary":"3 categorias"}]"#;
        let entries: Vec<crate::storage::HistoryEntry> = serde_json::from_str(old).unwrap();
        assert_eq!(EN.history_date(&entries[0].date), "04/11/2026 16:00");
        assert_eq!(EN.history_summary(&entries[0].summary), "3 categorias");
    }

    #[test]
    fn reads_structured_history_entries() {
        let new = r#"[{"date":{"year":2026,"month":11,"day":4,"hour":16,"minute":0},"kind":"GameMode","freed":0,"summary":{"game":"Hades","seconds":3725}}]"#;
        let entries: Vec<crate::storage::HistoryEntry> = serde_json::from_str(new).unwrap();
        assert_eq!(EN.history_date(&entries[0].date), "2026-11-04 16:00");
        assert_eq!(EN.history_summary(&entries[0].summary), "Hades for 01:02:05");
        assert_eq!(PT_BR.history_summary(&entries[0].summary), "Hades por 01:02:05");
    }
}
