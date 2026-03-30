use crate::{auth, cursor};
use ab_glyph::{point, Font, FontArc, GlyphId, PxScale, ScaleFont};
use anyhow::{Context, Result};
use chrono::{Datelike, Duration, Local, NaiveDate};
use colored::Colorize;
use image::{imageops::FilterType, Rgba, RgbaImage};
use imageproc::drawing::draw_filled_circle_mut;
use std::cmp::Ordering;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use tokio::runtime::Runtime;
use tokscale_core::{
    generate_graph, parse_local_clients, ClientId, GroupBy, LocalParseOptions, ReportOptions,
};

const SCALE: i32 = 2;
const IMAGE_WIDTH: i32 = 1200 * SCALE;
const IMAGE_HEIGHT: i32 = 1200 * SCALE;
const PADDING: i32 = 56 * SCALE;

const TOKSCALE_LOGO_SVG_URL: &str = "https://tokscale.ai/tokscale-logo.svg";
const TOKSCALE_LOGO_PNG_SIZE: i32 = 400;
const FIGTREE_REGULAR_FILE: &str = "Figtree-Regular.ttf";
const FIGTREE_REGULAR_URL: &str =
    "https://fonts.gstatic.com/s/figtree/v9/_Xmz-HUzqDCFdgfMsYiV_F7wfS-Bs_d_QF5e.ttf";
const FIGTREE_BOLD_FILE: &str = "Figtree-Bold.ttf";
const FIGTREE_BOLD_URL: &str =
    "https://fonts.gstatic.com/s/figtree/v9/_Xmz-HUzqDCFdgfMsYiV_F7wfS-Bs_eYR15e.ttf";

const PINNED_AGENTS: [&str; 2] = ["Sisyphus", "Planner-Sisyphus"];

const COLOR_BACKGROUND: Rgba<u8> = Rgba([0x10, 0x12, 0x1C, 0xFF]);
const COLOR_TEXT_PRIMARY: Rgba<u8> = Rgba([0xFF, 0xFF, 0xFF, 0xFF]);
const COLOR_TEXT_SECONDARY: Rgba<u8> = Rgba([0x88, 0x88, 0x88, 0xFF]);
const COLOR_GRADE0: Rgba<u8> = Rgba([0x14, 0x1A, 0x25, 0xFF]);
const COLOR_GRADE1: Rgba<u8> = Rgba([0x00, 0xB2, 0xFF, 0x44]);
const COLOR_GRADE2: Rgba<u8> = Rgba([0x00, 0xB2, 0xFF, 0x88]);
const COLOR_GRADE3: Rgba<u8> = Rgba([0x00, 0xB2, 0xFF, 0xCC]);
const COLOR_GRADE4: Rgba<u8> = Rgba([0x00, 0xB2, 0xFF, 0xFF]);
const COLOR_SISYPHUS: Rgba<u8> = Rgba([0x00, 0xCE, 0xD1, 0xFF]);

#[derive(Debug, Clone)]
pub struct WrappedOptions {
    pub output: Option<String>,
    pub year: Option<String>,
    pub clients: Option<Vec<String>>,
    pub short: bool,
    pub include_agents: bool,
    pub pin_sisyphus: bool,
}

#[derive(Debug, Clone)]
struct WrappedData {
    year: String,
    active_days: i32,
    total_tokens: i64,
    total_cost: f64,
    longest_streak: i32,
    top_models: Vec<WrappedRankedEntry>,
    top_clients: Vec<WrappedRankedEntry>,
    top_agents: Option<Vec<WrappedAgentEntry>>,
    contributions: Vec<WrappedContribution>,
    total_messages: i32,
}

#[derive(Debug, Clone)]
struct WrappedRankedEntry {
    name: String,
    cost: f64,
    tokens: i64,
}

#[derive(Debug, Clone)]
struct WrappedAgentEntry {
    name: String,
    tokens: i64,
    messages: i32,
}

#[derive(Debug, Clone)]
struct WrappedContribution {
    date: String,
    level: u8,
}

#[derive(Debug, Clone)]
struct FontSet {
    regular: FontArc,
    bold: FontArc,
}

#[derive(Debug, Clone)]
struct RenderOptions {
    short: bool,
    include_agents: bool,
    pin_sisyphus: bool,
}

pub fn run(options: WrappedOptions) -> Result<String> {
    let rt = Runtime::new()?;
    rt.block_on(async move { generate_wrapped(options).await })
}

async fn generate_wrapped(options: WrappedOptions) -> Result<String> {
    let data = load_wrapped_data(&options).await?;

    let agents_requested = options.include_agents;
    let has_agent_data = data
        .top_agents
        .as_ref()
        .map(|agents| !agents.is_empty())
        .unwrap_or(false);
    let opencode_enabled = options
        .clients
        .as_ref()
        .is_none_or(|clients| clients.iter().any(|s| s == "opencode"));
    let effective_include_agents = agents_requested && has_agent_data;

    if agents_requested && opencode_enabled && !has_agent_data {
        println!(
            "{}",
            format!("\n  ⚠ No OpenCode agent data found for {}.", data.year).yellow()
        );
        println!("{}", "    Falling back to clients view.".bright_black());
        println!(
            "{}",
            "    Use --clients to always show clients view.\n".bright_black()
        );
    }

    let image = generate_wrapped_image(
        &data,
        &RenderOptions {
            short: options.short,
            include_agents: effective_include_agents,
            pin_sisyphus: options.pin_sisyphus,
        },
    )
    .await?;

    let output = options
        .output
        .clone()
        .unwrap_or_else(|| format!("tokscale-{}-wrapped.png", data.year));
    let output_path = PathBuf::from(&output);
    let absolute = if output_path.is_absolute() {
        output_path
    } else {
        std::env::current_dir()?.join(output_path)
    };

    image
        .save_with_format(&absolute, image::ImageFormat::Png)
        .with_context(|| format!("Failed to save wrapped image to {}", absolute.display()))?;

    Ok(absolute.to_string_lossy().to_string())
}

async fn load_wrapped_data(options: &WrappedOptions) -> Result<WrappedData> {
    let year = options
        .year
        .clone()
        .unwrap_or_else(|| Local::now().year().to_string());
    let clients = options.clients.clone().unwrap_or_else(default_clients);
    let local_clients: Vec<String> = clients
        .iter()
        .filter(|src| src.as_str() != ClientId::Cursor.as_str())
        .cloned()
        .collect();
    let include_cursor = clients.iter().any(|src| src == ClientId::Cursor.as_str());

    let since = format!("{}-01-01", year);
    let until = format!("{}-12-31", year);

    let has_cursor_cache = cursor::has_cursor_usage_cache();
    let mut cursor_sync_result: Option<cursor::SyncCursorResult> = None;

    if include_cursor && cursor::is_cursor_logged_in() {
        cursor_sync_result = Some(cursor::sync_cursor_cache().await);
    }

    if let Some(sync) = cursor_sync_result.as_ref() {
        if let Some(error) = sync.error.as_ref() {
            if sync.synced || has_cursor_cache {
                let prefix = if sync.synced {
                    "Cursor sync warning"
                } else {
                    "Cursor sync failed; using cached data"
                };
                println!("{}", format!("  {}: {}", prefix, error).yellow());
            }
        }
    }

    let include_cursor_in_graph = if include_cursor {
        let synced = cursor_sync_result
            .as_ref()
            .map(|sync| sync.synced)
            .unwrap_or(false);
        synced || has_cursor_cache
    } else {
        false
    };

    let graph_clients = if include_cursor && !include_cursor_in_graph {
        clients
            .iter()
            .filter(|src| src.as_str() != ClientId::Cursor.as_str())
            .cloned()
            .collect::<Vec<_>>()
    } else {
        clients.clone()
    };

    let parsed_local = if options.include_agents && !local_clients.is_empty() {
        Some(
            parse_local_clients(LocalParseOptions {
                home_dir: None,
                clients: Some(local_clients),
                since: Some(since.clone()),
                until: Some(until.clone()),
                year: Some(year.clone()),
            })
            .map_err(anyhow::Error::msg)?,
        )
    } else {
        None
    };

    let graph = generate_graph(ReportOptions {
        home_dir: None,
        clients: Some(graph_clients),
        since: Some(since),
        until: Some(until),
        year: Some(year.clone()),
        group_by: GroupBy::default(),
    })
    .await
    .map_err(anyhow::Error::msg)?;

    let mut model_map: HashMap<String, WrappedRankedEntry> = HashMap::new();
    let mut client_map: HashMap<String, WrappedRankedEntry> = HashMap::new();
    let mut total_messages = 0i32;

    for day in &graph.contributions {
        total_messages += day.totals.messages;

        for client_contrib in &day.clients {
            let model_name = format_model_name(&client_contrib.model_id);
            let model_entry =
                model_map
                    .entry(model_name.clone())
                    .or_insert_with(|| WrappedRankedEntry {
                        name: model_name,
                        cost: 0.0,
                        tokens: 0,
                    });
            model_entry.cost += client_contrib.cost;
            model_entry.tokens += client_contrib.tokens.input
                + client_contrib.tokens.output
                + client_contrib.tokens.cache_read
                + client_contrib.tokens.cache_write;

            let client_name = client_display_name(&client_contrib.client)
                .unwrap_or(client_contrib.client.as_str())
                .to_string();
            let client_entry =
                client_map
                    .entry(client_name.clone())
                    .or_insert_with(|| WrappedRankedEntry {
                        name: client_name,
                        cost: 0.0,
                        tokens: 0,
                    });
            client_entry.cost += client_contrib.cost;
            client_entry.tokens += client_contrib.tokens.input
                + client_contrib.tokens.output
                + client_contrib.tokens.cache_read
                + client_contrib.tokens.cache_write;
        }
    }

    let mut top_models: Vec<WrappedRankedEntry> = model_map.into_values().collect();
    top_models.sort_by(|a, b| b.cost.partial_cmp(&a.cost).unwrap_or(Ordering::Equal));
    top_models.truncate(3);

    let mut top_clients: Vec<WrappedRankedEntry> = client_map.into_values().collect();
    top_clients.sort_by(|a, b| b.cost.partial_cmp(&a.cost).unwrap_or(Ordering::Equal));
    top_clients.truncate(3);

    let top_agents = if options.include_agents {
        parsed_local
            .as_ref()
            .map(build_top_agents)
            .filter(|agents| !agents.is_empty())
    } else {
        None
    };

    let max_cost = graph
        .contributions
        .iter()
        .map(|c| c.totals.cost)
        .fold(1.0, f64::max);
    let contributions: Vec<WrappedContribution> = graph
        .contributions
        .iter()
        .map(|c| WrappedContribution {
            date: c.date.clone(),
            level: calculate_intensity(c.totals.cost, max_cost),
        })
        .collect();

    let mut sorted_dates: Vec<String> = contributions
        .iter()
        .map(|c| c.date.clone())
        .filter(|date| date.starts_with(&year))
        .collect();
    sorted_dates.sort();

    let (_current_streak, longest_streak) = calculate_streaks(&sorted_dates);
    let _first_day = sorted_dates
        .first()
        .cloned()
        .unwrap_or_else(|| format!("{}-01-01", year));

    Ok(WrappedData {
        year,
        active_days: graph.summary.active_days,
        total_tokens: graph.summary.total_tokens,
        total_cost: graph.summary.total_cost,
        longest_streak,
        top_models,
        top_clients,
        top_agents,
        contributions,
        total_messages,
    })
}

fn build_top_agents(parsed: &tokscale_core::ParsedMessages) -> Vec<WrappedAgentEntry> {
    let mut agent_map: HashMap<String, WrappedAgentEntry> = HashMap::new();

    for message in &parsed.messages {
        if message.client != "opencode" {
            continue;
        }

        let Some(agent) = message.agent.as_ref() else {
            continue;
        };

        let normalized = tokscale_core::sessions::normalize_opencode_agent_name(agent);
        let tokens = message.input
            + message.output
            + message.cache_read
            + message.cache_write
            + message.reasoning;

        let entry = agent_map
            .entry(normalized.clone())
            .or_insert_with(|| WrappedAgentEntry {
                name: normalized,
                tokens: 0,
                messages: 0,
            });
        entry.tokens += tokens;
        entry.messages += 1;
    }

    let mut agents: Vec<WrappedAgentEntry> = agent_map.into_values().collect();

    let mut pinned = agents
        .iter()
        .filter(|agent| PINNED_AGENTS.contains(&agent.name.as_str()))
        .cloned()
        .collect::<Vec<_>>();
    let mut unpinned = agents
        .drain(..)
        .filter(|agent| !PINNED_AGENTS.contains(&agent.name.as_str()))
        .collect::<Vec<_>>();

    pinned.sort_by_key(|agent| {
        PINNED_AGENTS
            .iter()
            .position(|name| *name == agent.name)
            .unwrap_or(usize::MAX)
    });
    unpinned.sort_by(|a, b| b.messages.cmp(&a.messages));

    let mut combined = Vec::new();
    combined.extend(pinned);
    combined.extend(unpinned.into_iter().take(2));
    combined
}

async fn generate_wrapped_image(data: &WrappedData, options: &RenderOptions) -> Result<RgbaImage> {
    let client = reqwest::Client::new();
    let fonts = ensure_fonts_loaded(&client).await?;

    let mut canvas =
        RgbaImage::from_pixel(IMAGE_WIDTH as u32, IMAGE_HEIGHT as u32, COLOR_BACKGROUND);

    let left_width = (IMAGE_WIDTH as f32 * 0.45) as i32;
    let right_width = (IMAGE_WIDTH as f32 * 0.55) as i32;
    let right_x = left_width;

    let mut y_pos = PADDING + 24 * SCALE;

    let credentials = auth::load_credentials();
    let display_username = credentials
        .as_ref()
        .and_then(|cred| truncate_username(&cred.username, 30));
    let title_text = display_username
        .map(|username| format!("@{}'s Wrapped {}", username, data.year))
        .unwrap_or_else(|| format!("My Wrapped {}", data.year));

    draw_text_mut_baseline(
        &mut canvas,
        &fonts.bold,
        (28 * SCALE) as f32,
        COLOR_TEXT_PRIMARY,
        PADDING,
        y_pos,
        &title_text,
    );
    y_pos += 60 * SCALE;

    draw_text_mut_baseline(
        &mut canvas,
        &fonts.regular,
        (20 * SCALE) as f32,
        COLOR_TEXT_SECONDARY,
        PADDING,
        y_pos,
        "Total Tokens",
    );
    y_pos += 64 * SCALE;

    let total_tokens_display = if options.short {
        format_tokens_short(data.total_tokens)
    } else {
        format_number_with_commas_i64(data.total_tokens)
    };
    draw_text_mut_baseline(
        &mut canvas,
        &fonts.bold,
        (56 * SCALE) as f32,
        COLOR_GRADE4,
        PADDING,
        y_pos,
        &total_tokens_display,
    );
    y_pos += 50 * SCALE + 40 * SCALE;

    let logo_size = 32 * SCALE;
    let logo_radius = 6 * SCALE;

    draw_text_mut_baseline(
        &mut canvas,
        &fonts.regular,
        (20 * SCALE) as f32,
        COLOR_TEXT_SECONDARY,
        PADDING,
        y_pos,
        "Top Models",
    );
    y_pos += 48 * SCALE;

    for (index, model) in data.top_models.iter().enumerate() {
        draw_text_mut_baseline(
            &mut canvas,
            &fonts.bold,
            (32 * SCALE) as f32,
            COLOR_TEXT_PRIMARY,
            PADDING,
            y_pos,
            &(index + 1).to_string(),
        );

        let mut text_x = PADDING + 40 * SCALE;

        if let Some(provider) = get_provider_from_model(&model.name) {
            if let Some(logo_url) = provider_logo_url(provider) {
                let filename = format!("provider-{}@2x.jpg", provider);
                if let Ok(path) = fetch_and_cache_image(&client, logo_url, &filename).await {
                    if let Ok(logo) = load_rgba_image(&path) {
                        let logo_y = y_pos - logo_size + 6 * SCALE;
                        let logo_x = PADDING + 40 * SCALE;

                        draw_image_rounded(
                            &mut canvas,
                            &logo,
                            logo_x,
                            logo_y,
                            logo_size,
                            logo_size,
                            logo_radius,
                        );
                        draw_rounded_border(
                            &mut canvas,
                            logo_x,
                            logo_y,
                            logo_size,
                            logo_size,
                            logo_radius,
                            SCALE,
                            COLOR_GRADE0,
                        );

                        text_x = logo_x + logo_size + 12 * SCALE;
                    }
                }
            }
        }

        draw_text_mut_baseline(
            &mut canvas,
            &fonts.regular,
            (32 * SCALE) as f32,
            COLOR_TEXT_PRIMARY,
            text_x,
            y_pos,
            &model.name,
        );
        y_pos += 50 * SCALE;
    }
    y_pos += 40 * SCALE;

    if options.include_agents {
        draw_text_mut_baseline(
            &mut canvas,
            &fonts.regular,
            (20 * SCALE) as f32,
            COLOR_TEXT_SECONDARY,
            PADDING,
            y_pos,
            "Top OpenCode Agents",
        );
        y_pos += 48 * SCALE;

        let agents = data.top_agents.clone().unwrap_or_default();
        let mut rank_index = 1;

        for agent in agents {
            let is_sisyphus_agent = PINNED_AGENTS.contains(&agent.name.as_str());
            let show_with_dash = options.pin_sisyphus && is_sisyphus_agent;
            let prefix = if show_with_dash {
                "\u{2022}".to_string()
            } else {
                rank_index.to_string()
            };
            let prefix_color = if show_with_dash {
                COLOR_SISYPHUS
            } else {
                COLOR_TEXT_PRIMARY
            };

            draw_text_mut_baseline(
                &mut canvas,
                &fonts.bold,
                (32 * SCALE) as f32,
                prefix_color,
                PADDING,
                y_pos,
                &prefix,
            );

            if !show_with_dash {
                rank_index += 1;
            }

            let name_x = PADDING + 40 * SCALE;
            let name_color = if is_sisyphus_agent {
                COLOR_SISYPHUS
            } else {
                COLOR_TEXT_PRIMARY
            };
            draw_text_mut_baseline(
                &mut canvas,
                &fonts.regular,
                (32 * SCALE) as f32,
                name_color,
                name_x,
                y_pos,
                &agent.name,
            );

            let name_width = measure_text_width(&fonts.regular, (32 * SCALE) as f32, &agent.name);
            let suffix = format!(
                " ({})",
                format_number_with_commas_i64(agent.messages as i64)
            );
            draw_text_mut_baseline(
                &mut canvas,
                &fonts.regular,
                (32 * SCALE) as f32,
                COLOR_TEXT_SECONDARY,
                name_x + name_width.round() as i32,
                y_pos,
                &suffix,
            );

            y_pos += 50 * SCALE;
        }
    } else {
        draw_text_mut_baseline(
            &mut canvas,
            &fonts.regular,
            (20 * SCALE) as f32,
            COLOR_TEXT_SECONDARY,
            PADDING,
            y_pos,
            "Top Clients",
        );
        y_pos += 48 * SCALE;

        for (index, client_entry) in data.top_clients.iter().enumerate() {
            draw_text_mut_baseline(
                &mut canvas,
                &fonts.bold,
                (32 * SCALE) as f32,
                COLOR_TEXT_PRIMARY,
                PADDING,
                y_pos,
                &(index + 1).to_string(),
            );

            if let Some(logo_url) = client_logo_url(&client_entry.name) {
                let filename = format!(
                    "client-{}@2x.png",
                    client_entry
                        .name
                        .to_lowercase()
                        .split_whitespace()
                        .collect::<Vec<_>>()
                        .join("-")
                );

                if let Ok(path) = fetch_and_cache_image(&client, logo_url, &filename).await {
                    if let Ok(logo) = load_rgba_image(&path) {
                        let logo_x = PADDING + 40 * SCALE;
                        let logo_y = y_pos - logo_size + 6 * SCALE;

                        draw_image_rounded(
                            &mut canvas,
                            &logo,
                            logo_x,
                            logo_y,
                            logo_size,
                            logo_size,
                            logo_radius,
                        );
                        draw_rounded_border(
                            &mut canvas,
                            logo_x,
                            logo_y,
                            logo_size,
                            logo_size,
                            logo_radius,
                            SCALE,
                            COLOR_GRADE0,
                        );
                    }
                }
            }

            draw_text_mut_baseline(
                &mut canvas,
                &fonts.regular,
                (32 * SCALE) as f32,
                COLOR_TEXT_PRIMARY,
                PADDING + 40 * SCALE + logo_size + 12 * SCALE,
                y_pos,
                &client_entry.name,
            );
            y_pos += 50 * SCALE;
        }
    }

    y_pos += 40 * SCALE;

    let stats_start_y = y_pos;
    let stat_width = (left_width - PADDING * 2) / 2;

    draw_stat(
        &mut canvas,
        &fonts,
        PADDING,
        stats_start_y,
        "Messages",
        &format_number_with_commas_i64(data.total_messages as i64),
    );
    draw_stat(
        &mut canvas,
        &fonts,
        PADDING + stat_width,
        stats_start_y,
        "Active Days",
        &data.active_days.to_string(),
    );
    draw_stat(
        &mut canvas,
        &fonts,
        PADDING,
        stats_start_y + 100 * SCALE,
        "Cost",
        &format_cost(data.total_cost),
    );
    draw_stat(
        &mut canvas,
        &fonts,
        PADDING + stat_width,
        stats_start_y + 100 * SCALE,
        "Streak",
        &format!("{}d", data.longest_streak),
    );

    draw_contribution_graph(
        &mut canvas,
        data,
        right_x,
        PADDING,
        right_width - PADDING,
        IMAGE_HEIGHT - PADDING * 2,
    );

    let footer_bottom_y = IMAGE_HEIGHT - PADDING;
    let tokscale_logo_height = 72 * SCALE;

    if let Ok(logo_path) = fetch_svg_and_convert_to_png(
        &client,
        TOKSCALE_LOGO_SVG_URL,
        "tokscale-logo@2x.png",
        TOKSCALE_LOGO_PNG_SIZE * SCALE,
    )
    .await
    {
        if let Ok(logo) = load_rgba_image(&logo_path) {
            draw_text_mut_baseline(
                &mut canvas,
                &fonts.regular,
                (18 * SCALE) as f32,
                COLOR_TEXT_SECONDARY,
                PADDING,
                footer_bottom_y,
                "github.com/junhoyeo/tokscale",
            );

            let logo_width = ((logo.width() as f32 / logo.height() as f32)
                * tokscale_logo_height as f32)
                .round() as i32;
            let logo_y = footer_bottom_y - 18 * SCALE - 16 * SCALE - tokscale_logo_height;
            draw_image(
                &mut canvas,
                &logo,
                PADDING,
                logo_y,
                logo_width,
                tokscale_logo_height,
            );
        }
    }

    Ok(canvas)
}

fn draw_contribution_graph(
    canvas: &mut RgbaImage,
    data: &WrappedData,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
) {
    let year = data
        .year
        .parse::<i32>()
        .unwrap_or_else(|_| Local::now().year());
    let Some(start_date) = NaiveDate::from_ymd_opt(year, 1, 1) else {
        return;
    };
    let Some(end_date) = NaiveDate::from_ymd_opt(year, 12, 31) else {
        return;
    };

    let contrib_map: HashMap<&str, u8> = data
        .contributions
        .iter()
        .map(|contribution| (contribution.date.as_str(), contribution.level))
        .collect();

    const DAYS_PER_ROW: i32 = 14;
    let total_days = (end_date - start_date).num_days() + 1;
    let total_rows = ((total_days + (DAYS_PER_ROW as i64) - 1) / DAYS_PER_ROW as i64) as i32;

    let cell_size = ((height as f32 / total_rows as f32).floor() as i32)
        .min((width as f32 / DAYS_PER_ROW as f32).floor() as i32)
        .max(1);
    let dot_radius = (((cell_size - 2 * SCALE) as f32) / 2.0).floor().max(1.0) as i32;

    let graph_width = DAYS_PER_ROW * cell_size;
    let _graph_height = total_rows * cell_size;
    let offset_x = x + (width - graph_width) / 2;
    let offset_y = y;

    let grade_colors = [
        COLOR_GRADE0,
        COLOR_GRADE1,
        COLOR_GRADE2,
        COLOR_GRADE3,
        COLOR_GRADE4,
    ];

    let mut current_date = start_date;
    let mut day_index = 0i32;

    while current_date <= end_date {
        let date_key = current_date.format("%Y-%m-%d").to_string();
        let level = *contrib_map.get(date_key.as_str()).unwrap_or(&0);

        let col = day_index % DAYS_PER_ROW;
        let row = day_index / DAYS_PER_ROW;

        let center_x = offset_x + col * cell_size + cell_size / 2;
        let center_y = offset_y + row * cell_size + cell_size / 2;

        draw_filled_circle_mut(
            canvas,
            (center_x, center_y),
            dot_radius,
            grade_colors[level as usize],
        );

        current_date += Duration::days(1);
        day_index += 1;
    }
}

fn draw_stat(canvas: &mut RgbaImage, fonts: &FontSet, x: i32, y: i32, label: &str, value: &str) {
    draw_text_mut_baseline(
        canvas,
        &fonts.regular,
        (18 * SCALE) as f32,
        COLOR_TEXT_SECONDARY,
        x,
        y,
        label,
    );
    draw_text_mut_baseline(
        canvas,
        &fonts.bold,
        (36 * SCALE) as f32,
        COLOR_TEXT_PRIMARY,
        x,
        y + 48 * SCALE,
        value,
    );
}

fn draw_text_mut_baseline(
    canvas: &mut RgbaImage,
    font: &FontArc,
    font_size: f32,
    color: Rgba<u8>,
    x: i32,
    baseline_y: i32,
    text: &str,
) {
    let scale = PxScale::from(font_size);
    let scaled_font = font.as_scaled(scale);

    let mut caret_x = x as f32;
    let baseline = baseline_y as f32;
    let mut prev_glyph: Option<GlyphId> = None;

    for ch in text.chars() {
        let glyph_id = scaled_font.glyph_id(ch);
        if let Some(prev) = prev_glyph {
            caret_x += scaled_font.kern(prev, glyph_id);
        }

        let glyph = glyph_id.with_scale_and_position(scale, point(caret_x, baseline));
        if let Some(outlined) = font.outline_glyph(glyph) {
            let bounds = outlined.px_bounds();
            outlined.draw(|gx, gy, coverage| {
                let px = bounds.min.x as i32 + gx as i32;
                let py = bounds.min.y as i32 + gy as i32;
                blend_pixel_with_coverage(canvas, px, py, color, coverage);
            });
        }

        caret_x += scaled_font.h_advance(glyph_id);
        prev_glyph = Some(glyph_id);
    }
}

fn measure_text_width(font: &FontArc, font_size: f32, text: &str) -> f32 {
    let scale = PxScale::from(font_size);
    let scaled_font = font.as_scaled(scale);
    let mut width = 0.0f32;
    let mut prev_glyph: Option<GlyphId> = None;

    for ch in text.chars() {
        let glyph_id = scaled_font.glyph_id(ch);
        if let Some(prev) = prev_glyph {
            width += scaled_font.kern(prev, glyph_id);
        }
        width += scaled_font.h_advance(glyph_id);
        prev_glyph = Some(glyph_id);
    }

    width
}

fn draw_image(canvas: &mut RgbaImage, source: &RgbaImage, x: i32, y: i32, width: i32, height: i32) {
    if width <= 0 || height <= 0 {
        return;
    }

    let resized =
        image::imageops::resize(source, width as u32, height as u32, FilterType::CatmullRom);
    for dy in 0..height {
        for dx in 0..width {
            let src = *resized.get_pixel(dx as u32, dy as u32);
            blend_pixel(canvas, x + dx, y + dy, src);
        }
    }
}

fn draw_image_rounded(
    canvas: &mut RgbaImage,
    source: &RgbaImage,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    radius: i32,
) {
    if width <= 0 || height <= 0 {
        return;
    }

    let resized =
        image::imageops::resize(source, width as u32, height as u32, FilterType::CatmullRom);
    for dy in 0..height {
        for dx in 0..width {
            let px = x + dx;
            let py = y + dy;
            if point_in_rounded_rect(
                px as f32 + 0.5,
                py as f32 + 0.5,
                x as f32,
                y as f32,
                width as f32,
                height as f32,
                radius as f32,
            ) {
                let src = *resized.get_pixel(dx as u32, dy as u32);
                blend_pixel(canvas, px, py, src);
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_rounded_border(
    canvas: &mut RgbaImage,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    radius: i32,
    line_width: i32,
    color: Rgba<u8>,
) {
    if width <= 0 || height <= 0 || line_width <= 0 {
        return;
    }

    for py in y..(y + height) {
        for px in x..(x + width) {
            let cx = px as f32 + 0.5;
            let cy = py as f32 + 0.5;
            let outer = point_in_rounded_rect(
                cx,
                cy,
                x as f32,
                y as f32,
                width as f32,
                height as f32,
                radius as f32,
            );
            if !outer {
                continue;
            }

            let inner = point_in_rounded_rect(
                cx,
                cy,
                (x + line_width) as f32,
                (y + line_width) as f32,
                (width - 2 * line_width) as f32,
                (height - 2 * line_width) as f32,
                (radius - line_width).max(0) as f32,
            );

            if !inner {
                blend_pixel(canvas, px, py, color);
            }
        }
    }
}

fn point_in_rounded_rect(
    px: f32,
    py: f32,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    radius: f32,
) -> bool {
    if width <= 0.0 || height <= 0.0 {
        return false;
    }

    let r = radius.max(0.0).min(width / 2.0).min(height / 2.0);
    if r <= 0.0 {
        return px >= x && px < x + width && py >= y && py < y + height;
    }

    let nearest_x = px.clamp(x + r, x + width - r);
    let nearest_y = py.clamp(y + r, y + height - r);
    let dx = px - nearest_x;
    let dy = py - nearest_y;
    dx * dx + dy * dy <= r * r
}

fn blend_pixel_with_coverage(
    canvas: &mut RgbaImage,
    x: i32,
    y: i32,
    color: Rgba<u8>,
    coverage: f32,
) {
    let mut src = color;
    src.0[3] = ((src.0[3] as f32) * coverage.clamp(0.0, 1.0)).round() as u8;
    blend_pixel(canvas, x, y, src);
}

fn blend_pixel(canvas: &mut RgbaImage, x: i32, y: i32, src: Rgba<u8>) {
    if x < 0 || y < 0 {
        return;
    }

    let width = canvas.width() as i32;
    let height = canvas.height() as i32;
    if x >= width || y >= height {
        return;
    }

    let dst = canvas.get_pixel_mut(x as u32, y as u32);
    let src_alpha = src.0[3] as f32 / 255.0;
    if src_alpha <= 0.0 {
        return;
    }

    let dst_alpha = dst.0[3] as f32 / 255.0;
    let out_alpha = src_alpha + dst_alpha * (1.0 - src_alpha);

    if out_alpha <= 0.0 {
        *dst = Rgba([0, 0, 0, 0]);
        return;
    }

    for channel in 0..3 {
        let src_channel = src.0[channel] as f32 / 255.0;
        let dst_channel = dst.0[channel] as f32 / 255.0;
        let out_channel =
            (src_channel * src_alpha + dst_channel * dst_alpha * (1.0 - src_alpha)) / out_alpha;
        dst.0[channel] = (out_channel * 255.0).round().clamp(0.0, 255.0) as u8;
    }

    dst.0[3] = (out_alpha * 255.0).round().clamp(0.0, 255.0) as u8;
}

async fn ensure_fonts_loaded(client: &reqwest::Client) -> Result<FontSet> {
    let cache_dir = get_font_cache_dir()?;
    ensure_cache_dir(&cache_dir)?;

    let regular_path = cache_dir.join(FIGTREE_REGULAR_FILE);
    let bold_path = cache_dir.join(FIGTREE_BOLD_FILE);

    if !regular_path.exists() {
        let _ = fetch_to_file(client, FIGTREE_REGULAR_URL, &regular_path).await;
    }
    if !bold_path.exists() {
        let _ = fetch_to_file(client, FIGTREE_BOLD_URL, &bold_path).await;
    }

    let regular_font = if regular_path.exists() {
        fs::read(&regular_path)
            .ok()
            .and_then(|bytes| FontArc::try_from_vec(bytes).ok())
    } else {
        None
    };
    let bold_font = if bold_path.exists() {
        fs::read(&bold_path)
            .ok()
            .and_then(|bytes| FontArc::try_from_vec(bytes).ok())
    } else {
        None
    };

    let (regular, bold) = match (regular_font, bold_font) {
        (Some(regular), Some(bold)) => (regular, bold),
        (Some(regular), None) => (regular.clone(), regular),
        (None, Some(bold)) => (bold.clone(), bold),
        (None, None) => {
            anyhow::bail!(
                "Failed to load Figtree fonts. Could not download or parse cached font files."
            )
        }
    };

    Ok(FontSet { regular, bold })
}

async fn fetch_and_cache_image(
    client: &reqwest::Client,
    url: &str,
    filename: &str,
) -> Result<PathBuf> {
    let cache_dir = get_image_cache_dir()?;
    ensure_cache_dir(&cache_dir)?;

    let cached_path = cache_dir.join(filename);
    if !cached_path.exists() {
        fetch_to_file(client, url, &cached_path).await?;
    }

    Ok(cached_path)
}

async fn fetch_svg_and_convert_to_png(
    client: &reqwest::Client,
    svg_url: &str,
    filename: &str,
    size: i32,
) -> Result<PathBuf> {
    let cache_dir = get_image_cache_dir()?;
    ensure_cache_dir(&cache_dir)?;

    let cached_path = cache_dir.join(filename);
    if cached_path.exists() {
        return Ok(cached_path);
    }

    let response = client
        .get(svg_url)
        .send()
        .await
        .with_context(|| format!("Failed to fetch {}", svg_url))?;
    if !response.status().is_success() {
        anyhow::bail!("Failed to fetch {} (status {})", svg_url, response.status());
    }

    let svg_bytes = response.bytes().await?;
    let options = usvg::Options::default();
    let tree = usvg::Tree::from_data(&svg_bytes, &options)
        .map_err(|err| anyhow::anyhow!("Failed to parse SVG: {err:?}"))?;

    let base_size = tree.size().to_int_size();
    let scale = size as f32 / base_size.width() as f32;
    let scaled_size = base_size
        .scale_by(scale)
        .ok_or_else(|| anyhow::anyhow!("Invalid scaled SVG size"))?;

    let mut pixmap = resvg::tiny_skia::Pixmap::new(scaled_size.width(), scaled_size.height())
        .ok_or_else(|| anyhow::anyhow!("Failed to create pixmap"))?;
    let transform = resvg::tiny_skia::Transform::from_scale(scale, scale);
    resvg::render(&tree, transform, &mut pixmap.as_mut());

    let png = pixmap
        .encode_png()
        .map_err(|err| anyhow::anyhow!("Failed to encode PNG: {err:?}"))?;
    fs::write(&cached_path, png)?;

    Ok(cached_path)
}

async fn fetch_to_file(client: &reqwest::Client, url: &str, path: &Path) -> Result<()> {
    let response = client
        .get(url)
        .send()
        .await
        .with_context(|| format!("Failed to fetch {}", url))?;

    if !response.status().is_success() {
        anyhow::bail!("Failed to fetch {} (status {})", url, response.status());
    }

    let bytes = response.bytes().await?;
    fs::write(path, &bytes)?;
    Ok(())
}

fn ensure_cache_dir(path: &Path) -> Result<()> {
    if !path.exists() {
        fs::create_dir_all(path)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
        }
    }
    Ok(())
}

fn load_rgba_image(path: &Path) -> Result<RgbaImage> {
    Ok(image::open(path)
        .with_context(|| format!("Failed to decode image {}", path.display()))?
        .to_rgba8())
}

fn get_image_cache_dir() -> Result<PathBuf> {
    let home = dirs::home_dir().context("Could not determine home directory")?;
    Ok(home.join(".cache").join("tokscale").join("images"))
}

fn get_font_cache_dir() -> Result<PathBuf> {
    let home = dirs::home_dir().context("Could not determine home directory")?;
    Ok(home.join(".cache").join("tokscale").join("fonts"))
}

fn calculate_intensity(cost: f64, max_cost: f64) -> u8 {
    if cost == 0.0 || max_cost == 0.0 {
        return 0;
    }

    let ratio = cost / max_cost;
    if ratio >= 0.75 {
        4
    } else if ratio >= 0.5 {
        3
    } else if ratio >= 0.25 {
        2
    } else {
        1
    }
}

fn calculate_streaks(sorted_dates: &[String]) -> (i32, i32) {
    let today = Local::now().date_naive().format("%Y-%m-%d").to_string();
    calculate_streaks_with_today(sorted_dates, &today)
}

fn calculate_streaks_with_today(sorted_dates: &[String], today: &str) -> (i32, i32) {
    if sorted_dates.is_empty() {
        return (0, 0);
    }

    let mut current_streak = 0;
    let mut longest_streak = 0;
    let mut streak = 1;

    for index in (0..sorted_dates.len()).rev() {
        if index == sorted_dates.len() - 1 {
            let days_diff = date_diff_days(&sorted_dates[index], today);
            if days_diff <= 1 {
                current_streak = 1;
            } else {
                break;
            }
        } else {
            let days_diff = date_diff_days(&sorted_dates[index], &sorted_dates[index + 1]);
            if days_diff == 1 {
                current_streak += 1;
            } else {
                break;
            }
        }
    }

    for index in 1..sorted_dates.len() {
        let days_diff = date_diff_days(&sorted_dates[index - 1], &sorted_dates[index]);
        if days_diff == 1 {
            streak += 1;
        } else {
            longest_streak = longest_streak.max(streak);
            streak = 1;
        }
    }
    longest_streak = longest_streak.max(streak);

    (current_streak, longest_streak)
}

fn date_diff_days(date1: &str, date2: &str) -> i64 {
    let parsed1 = NaiveDate::parse_from_str(date1, "%Y-%m-%d");
    let parsed2 = NaiveDate::parse_from_str(date2, "%Y-%m-%d");

    match (parsed1, parsed2) {
        (Ok(d1), Ok(d2)) => (d2 - d1).num_days().abs(),
        _ => 0,
    }
}

fn format_tokens_short(tokens: i64) -> String {
    if tokens >= 1_000_000_000 {
        format!("{:.2}B", tokens as f64 / 1_000_000_000.0)
    } else if tokens >= 1_000_000 {
        format!("{:.2}M", tokens as f64 / 1_000_000.0)
    } else if tokens >= 1_000 {
        format!("{:.1}K", tokens as f64 / 1_000.0)
    } else {
        tokens.to_string()
    }
}

fn format_cost(cost: f64) -> String {
    if cost >= 1000.0 {
        format!("${:.2}K", cost / 1000.0)
    } else {
        format!("${:.2}", cost)
    }
}

fn format_number_with_commas_i64(value: i64) -> String {
    let sign = if value < 0 { "-" } else { "" };
    let digits = value.abs().to_string();
    let mut result = String::with_capacity(digits.len() + digits.len() / 3 + sign.len());

    result.push_str(sign);
    for (index, ch) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            result.push(',');
        }
        result.push(ch);
    }

    result
}

fn client_display_name(client: &str) -> Option<&'static str> {
    match client {
        "opencode" => Some("OpenCode"),
        "claude" => Some("Claude Code"),
        "codex" => Some("Codex CLI"),
        "gemini" => Some("Gemini CLI"),
        s if s == ClientId::Cursor.as_str() => Some("Cursor IDE"),
        "amp" => Some("Amp"),
        "droid" => Some("Droid"),
        "openclaw" => Some("OpenClaw"),
        "pi" => Some("Pi"),
        "kimi" => Some("Kimi CLI"),
        "qwen" => Some("Qwen CLI"),
        "roocode" => Some("Roo Code"),
        "kilo" => Some("Kilo"),
        "mux" => Some("Mux"),
        "synthetic" => Some("Synthetic"),
        _ => None,
    }
}

fn client_logo_url(client_name: &str) -> Option<&'static str> {
    match client_name {
        "OpenCode" => Some("https://tokscale.ai/assets/logos/opencode.png"),
        "Claude Code" => Some("https://tokscale.ai/assets/logos/claude.jpg"),
        "Codex CLI" => Some("https://tokscale.ai/assets/logos/openai.jpg"),
        "Gemini CLI" => Some("https://tokscale.ai/assets/logos/gemini.png"),
        "Cursor IDE" => Some("https://tokscale.ai/assets/logos/cursor.jpg"),
        "Amp" => Some("https://tokscale.ai/assets/logos/amp.png"),
        "Droid" => Some("https://tokscale.ai/assets/logos/droid.png"),
        "OpenClaw" => Some("https://tokscale.ai/assets/logos/openclaw.png"),
        "Pi" => Some("https://tokscale.ai/assets/logos/pi.png"),
        "Kimi CLI" => Some("https://tokscale.ai/assets/logos/kimi.png"),
        "Qwen CLI" => Some("https://tokscale.ai/assets/logos/qwen.png"),
        "Roo Code" => Some("https://tokscale.ai/assets/logos/roocode.png"),
        "Kilo" => Some("https://tokscale.ai/assets/logos/kilocode.png"),
        "Mux" => Some("https://tokscale.ai/assets/logos/mux.png"),
        "Synthetic" => Some("https://tokscale.ai/assets/logos/synthetic.png"),
        _ => None,
    }
}

fn provider_logo_url(provider: &str) -> Option<&'static str> {
    match provider {
        "anthropic" => Some("https://tokscale.ai/assets/logos/claude.jpg"),
        "openai" => Some("https://tokscale.ai/assets/logos/openai.jpg"),
        "google" => Some("https://tokscale.ai/assets/logos/gemini.png"),
        "xai" => Some("https://tokscale.ai/assets/logos/grok.jpg"),
        "zai" => Some("https://tokscale.ai/assets/logos/zai.jpg"),
        _ => None,
    }
}

fn get_provider_from_model(model_id: &str) -> Option<&'static str> {
    let lower = model_id.to_lowercase();
    if lower.contains("claude")
        || lower.contains("opus")
        || lower.contains("sonnet")
        || lower.contains("haiku")
    {
        return Some("anthropic");
    }
    if lower.contains("gpt")
        || lower.contains("o1")
        || lower.contains("o3")
        || lower.contains("codex")
    {
        return Some("openai");
    }
    if lower.contains("gemini") {
        return Some("google");
    }
    if lower.contains("grok") {
        return Some("xai");
    }
    if lower.contains("glm") || lower.contains("pickle") {
        return Some("zai");
    }
    None
}

fn format_model_name(model: &str) -> String {
    if let Some(display) = exact_model_display_name(model) {
        return display.to_string();
    }

    let (without_quality, suffix) = split_quality_suffix(model);
    let mut cleaned = without_quality;

    if let Some(index) = cleaned.rfind(':') {
        let tail = &cleaned[index + 1..];
        if !tail.is_empty()
            && tail
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            cleaned.truncate(index);
        }
    }

    if cleaned.to_lowercase().ends_with("-thinking") {
        cleaned.truncate(cleaned.len() - "-thinking".len());
    } else if cleaned.to_lowercase().ends_with("_thinking") {
        cleaned.truncate(cleaned.len() - "_thinking".len());
    }

    cleaned = strip_date_suffix(cleaned);

    let normalized = cleaned
        .to_lowercase()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect::<String>();

    if normalized.contains("claudeopus45") {
        return format!("Claude Opus 4.5{}", suffix);
    }
    if normalized.contains("claude4opus") {
        return format!("Claude 4 Opus{}", suffix);
    }
    if normalized.contains("claudeopus4") {
        return format!("Claude Opus 4{}", suffix);
    }
    if normalized.contains("claudesonnet45") {
        return format!("Claude Sonnet 4.5{}", suffix);
    }
    if normalized.contains("claude4sonnet") {
        return format!("Claude 4 Sonnet{}", suffix);
    }
    if normalized.contains("claudesonnet4") {
        return format!("Claude Sonnet 4{}", suffix);
    }
    if normalized.contains("claudehaiku45") {
        return format!("Claude Haiku 4.5{}", suffix);
    }
    if normalized.contains("claude4haiku") {
        return format!("Claude 4 Haiku{}", suffix);
    }
    if normalized.contains("claudehaiku4") {
        return format!("Claude Haiku 4{}", suffix);
    }
    if normalized.contains("claude37sonnet") {
        return format!("Claude 3.7 Sonnet{}", suffix);
    }
    if normalized.contains("claude35sonnet") {
        return format!("Claude 3.5 Sonnet{}", suffix);
    }
    if normalized.contains("claude35haiku") {
        return format!("Claude 3.5 Haiku{}", suffix);
    }
    if normalized.contains("claude3opus") {
        return format!("Claude 3 Opus{}", suffix);
    }
    if normalized.contains("claude3sonnet") {
        return format!("Claude 3 Sonnet{}", suffix);
    }
    if normalized.contains("claude3haiku") {
        return format!("Claude 3 Haiku{}", suffix);
    }
    if normalized.contains("gpt51") {
        return format!("GPT-5.1{}", suffix);
    }
    if normalized.contains("gpt5") {
        return format!("GPT-5{}", suffix);
    }
    if normalized.contains("gpt4omini") {
        return format!("GPT-4o Mini{}", suffix);
    }
    if normalized.contains("gpt4o") {
        return format!("GPT-4o{}", suffix);
    }
    if normalized.contains("gpt4turbo") {
        return format!("GPT-4 Turbo{}", suffix);
    }
    if normalized.contains("gpt4") {
        return format!("GPT-4{}", suffix);
    }
    if normalized.starts_with("o1mini") {
        return format!("o1 Mini{}", suffix);
    }
    if normalized.starts_with("o1preview") {
        return format!("o1 Preview{}", suffix);
    }
    if normalized.starts_with("o3mini") {
        return format!("o3 Mini{}", suffix);
    }
    if normalized == "o1" {
        return format!("o1{}", suffix);
    }
    if normalized == "o3" {
        return format!("o3{}", suffix);
    }
    if normalized.contains("gemini3pro") {
        return format!("Gemini 3 Pro{}", suffix);
    }
    if normalized.contains("gemini3flash") {
        return format!("Gemini 3 Flash{}", suffix);
    }
    if normalized.contains("gemini25pro") {
        return format!("Gemini 2.5 Pro{}", suffix);
    }
    if normalized.contains("gemini25flash") {
        return format!("Gemini 2.5 Flash{}", suffix);
    }
    if normalized.contains("gemini20flash") {
        return format!("Gemini 2.0 Flash{}", suffix);
    }
    if normalized.contains("gemini15pro") {
        return format!("Gemini 1.5 Pro{}", suffix);
    }
    if normalized.contains("gemini15flash") {
        return format!("Gemini 1.5 Flash{}", suffix);
    }
    if normalized.contains("grok3mini") {
        return format!("Grok Code 3 Mini{}", suffix);
    }
    if normalized.contains("grok3") {
        return format!("Grok Code 3{}", suffix);
    }
    if normalized.contains("grok") {
        return format!("Grok Code{}", suffix);
    }
    if normalized.contains("deepseekv3") {
        return format!("DeepSeek V3{}", suffix);
    }
    if normalized.contains("deepseekr1") {
        return format!("DeepSeek R1{}", suffix);
    }
    if normalized.contains("deepseek") {
        return format!("DeepSeek{}", suffix);
    }

    let mut fallback = cleaned;
    let fallback_lower = fallback.to_lowercase();
    if fallback_lower.starts_with("claude-") || fallback_lower.starts_with("claude_") {
        fallback = format!("Claude {}", &fallback[7..]);
    } else if fallback_lower.starts_with("gpt-") || fallback_lower.starts_with("gpt_") {
        fallback = format!("GPT-{}", &fallback[4..]);
    } else if fallback_lower.starts_with("gemini-") || fallback_lower.starts_with("gemini_") {
        fallback = format!("Gemini {}", &fallback[7..]);
    } else if fallback_lower.starts_with("grok-") || fallback_lower.starts_with("grok_") {
        fallback = format!("Grok Code {}", &fallback[5..]);
    }

    let base_name = fallback
        .split(['-', '_'])
        .filter(|word| !word.is_empty())
        .map(capitalize_word)
        .collect::<Vec<_>>()
        .join(" ")
        .trim()
        .to_string();

    if base_name.is_empty() {
        format!("{}{}", model, suffix)
    } else {
        format!("{}{}", base_name, suffix)
    }
}

fn exact_model_display_name(model: &str) -> Option<&'static str> {
    match model {
        "claude-sonnet-4-20250514" => Some("Claude Sonnet 4"),
        "claude-3-5-sonnet-20241022" => Some("Claude 3.5 Sonnet"),
        "claude-3-5-sonnet-20240620" => Some("Claude 3.5 Sonnet"),
        "claude-3-opus-20240229" => Some("Claude 3 Opus"),
        "claude-3-haiku-20240307" => Some("Claude 3 Haiku"),
        "gpt-4o" => Some("GPT-4o"),
        "gpt-4o-mini" => Some("GPT-4o Mini"),
        "gpt-4-turbo" => Some("GPT-4 Turbo"),
        "o1" => Some("o1"),
        "o1-mini" => Some("o1 Mini"),
        "o1-preview" => Some("o1 Preview"),
        "o3-mini" => Some("o3 Mini"),
        "gemini-2.5-pro" => Some("Gemini 2.5 Pro"),
        "gemini-2.5-flash" => Some("Gemini 2.5 Flash"),
        "gemini-2.0-flash" => Some("Gemini 2.0 Flash"),
        "gemini-1.5-pro" => Some("Gemini 1.5 Pro"),
        "gemini-1.5-flash" => Some("Gemini 1.5 Flash"),
        "grok-3" => Some("Grok 3"),
        "grok-3-mini" => Some("Grok 3 Mini"),
        _ => None,
    }
}

fn split_quality_suffix(model: &str) -> (String, String) {
    let lower = model.to_lowercase();

    for (needle, label) in [
        ("-high", " High"),
        ("_high", " High"),
        ("-medium", " Medium"),
        ("_medium", " Medium"),
        ("-low", " Low"),
        ("_low", " Low"),
    ] {
        if lower.ends_with(needle) {
            let base = model[..model.len() - needle.len()].to_string();
            return (base, label.to_string());
        }
    }

    (model.to_string(), String::new())
}

fn strip_date_suffix(mut model: String) -> String {
    let lower = model.to_lowercase();
    if lower.len() > 9 {
        if let Some(last_dash) = model.rfind('-') {
            let tail = &model[last_dash + 1..];
            if tail.len() == 8 && tail.chars().all(|ch| ch.is_ascii_digit()) {
                model.truncate(last_dash);
            }
        }
    }

    if let Some(last_dash) = model.rfind('-') {
        let tail = &model[last_dash + 1..];
        if tail.chars().all(|ch| ch.is_ascii_digit()) {
            if let Some(prev_dash) = model[..last_dash].rfind('-') {
                let prev_tail = &model[prev_dash + 1..last_dash];
                if prev_tail.starts_with("20")
                    && prev_tail.len() >= 8
                    && prev_tail.len() <= 10
                    && prev_tail.chars().all(|ch| ch.is_ascii_digit())
                {
                    model.truncate(prev_dash);
                }
            }
        }
    }

    model
}

fn capitalize_word(word: &str) -> String {
    if word.is_empty() {
        return String::new();
    }

    let mut chars = word.chars();
    let Some(first) = chars.next() else {
        return String::new();
    };

    let mut result = String::new();
    result.extend(first.to_uppercase());
    result.push_str(&chars.as_str().to_lowercase());
    result
}

fn truncate_username(username: &str, max_chars: usize) -> Option<String> {
    if username.is_empty() {
        return None;
    }

    let len = username.chars().count();
    if len <= max_chars {
        return Some(username.to_string());
    }

    if max_chars <= 1 {
        return Some("\u{2026}".to_string());
    }

    let truncated = username.chars().take(max_chars - 1).collect::<String>();
    Some(format!("{}\u{2026}", truncated))
}

fn default_clients() -> Vec<String> {
    vec![
        "opencode".to_string(),
        "claude".to_string(),
        "codex".to_string(),
        "gemini".to_string(),
        ClientId::Cursor.as_str().to_string(),
        "amp".to_string(),
        "droid".to_string(),
        "openclaw".to_string(),
        "pi".to_string(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    // ========== format_tokens_short tests ==========

    #[test]
    fn test_format_tokens_short_billions() {
        assert_eq!(format_tokens_short(1_500_000_000), "1.50B");
        assert_eq!(format_tokens_short(2_340_000_000), "2.34B");
        assert_eq!(format_tokens_short(10_000_000_000), "10.00B");
    }

    #[test]
    fn test_format_tokens_short_millions() {
        assert_eq!(format_tokens_short(234_000_000), "234.00M");
        assert_eq!(format_tokens_short(1_500_000), "1.50M");
        assert_eq!(format_tokens_short(999_999_999), "1000.00M");
    }

    #[test]
    fn test_format_tokens_short_thousands() {
        assert_eq!(format_tokens_short(5_678), "5.7K");
        assert_eq!(format_tokens_short(1_000), "1.0K");
        assert_eq!(format_tokens_short(999_999), "1000.0K");
    }

    #[test]
    fn test_format_tokens_short_small_numbers() {
        assert_eq!(format_tokens_short(123), "123");
        assert_eq!(format_tokens_short(0), "0");
        assert_eq!(format_tokens_short(999), "999");
    }

    // ========== format_cost tests ==========

    #[test]
    fn test_format_cost_standard() {
        assert_eq!(format_cost(12.34), "$12.34");
        assert_eq!(format_cost(0.99), "$0.99");
        assert_eq!(format_cost(100.00), "$100.00");
    }

    #[test]
    fn test_format_cost_thousands() {
        assert_eq!(format_cost(1234.56), "$1.23K");
        assert_eq!(format_cost(5000.00), "$5.00K");
        assert_eq!(format_cost(999.99), "$999.99");
    }

    #[test]
    fn test_format_cost_zero() {
        assert_eq!(format_cost(0.0), "$0.00");
    }

    // ========== format_number_with_commas_i64 tests ==========

    #[test]
    fn test_format_number_with_commas_i64_large() {
        assert_eq!(format_number_with_commas_i64(1_234_567), "1,234,567");
        assert_eq!(
            format_number_with_commas_i64(1_000_000_000),
            "1,000,000,000"
        );
    }

    #[test]
    fn test_format_number_with_commas_i64_small() {
        assert_eq!(format_number_with_commas_i64(100), "100");
        assert_eq!(format_number_with_commas_i64(999), "999");
    }

    #[test]
    fn test_format_number_with_commas_i64_negative() {
        assert_eq!(format_number_with_commas_i64(-1_234_567), "-1,234,567");
        assert_eq!(format_number_with_commas_i64(-100), "-100");
    }

    #[test]
    fn test_format_number_with_commas_i64_zero() {
        assert_eq!(format_number_with_commas_i64(0), "0");
    }

    // ========== truncate_username tests ==========

    #[test]
    fn test_truncate_username_long() {
        assert_eq!(
            truncate_username("very_long_username", 10),
            Some("very_long…".to_string())
        );
        assert_eq!(
            truncate_username("abcdefghijklmnop", 8),
            Some("abcdefg…".to_string())
        );
    }

    #[test]
    fn test_truncate_username_short() {
        assert_eq!(truncate_username("short", 10), Some("short".to_string()));
        assert_eq!(truncate_username("user", 4), Some("user".to_string()));
    }

    #[test]
    fn test_truncate_username_empty() {
        assert_eq!(truncate_username("", 10), None);
    }

    #[test]
    fn test_truncate_username_edge_cases() {
        assert_eq!(truncate_username("a", 1), Some("a".to_string()));
        assert_eq!(truncate_username("ab", 1), Some("…".to_string()));
    }

    // ========== capitalize_word tests ==========

    #[test]
    fn test_capitalize_word_lowercase() {
        assert_eq!(capitalize_word("hello"), "Hello");
        assert_eq!(capitalize_word("world"), "World");
    }

    #[test]
    fn test_capitalize_word_uppercase() {
        assert_eq!(capitalize_word("WORLD"), "World");
        assert_eq!(capitalize_word("HELLO"), "Hello");
    }

    #[test]
    fn test_capitalize_word_mixed() {
        assert_eq!(capitalize_word("hElLo"), "Hello");
        assert_eq!(capitalize_word("WoRlD"), "World");
    }

    #[test]
    fn test_capitalize_word_empty() {
        assert_eq!(capitalize_word(""), "");
    }

    #[test]
    fn test_capitalize_word_single_char() {
        assert_eq!(capitalize_word("a"), "A");
        assert_eq!(capitalize_word("Z"), "Z");
    }

    // ========== strip_date_suffix tests ==========

    #[test]
    fn test_strip_date_suffix_with_date() {
        assert_eq!(
            strip_date_suffix("claude-4-20250514".to_string()),
            "claude-4"
        );
        assert_eq!(strip_date_suffix("gpt-4-20240101".to_string()), "gpt-4");
    }

    #[test]
    fn test_strip_date_suffix_without_date() {
        assert_eq!(strip_date_suffix("gpt-5".to_string()), "gpt-5");
        assert_eq!(strip_date_suffix("claude-opus".to_string()), "claude-opus");
    }

    #[test]
    fn test_strip_date_suffix_complex() {
        assert_eq!(
            strip_date_suffix("model-2024-01-15-123".to_string()),
            "model-2024-01-15-123"
        );
    }

    // ========== split_quality_suffix tests ==========

    #[test]
    fn test_split_quality_suffix_high() {
        assert_eq!(
            split_quality_suffix("model-high"),
            ("model".to_string(), " High".to_string())
        );
        assert_eq!(
            split_quality_suffix("gpt-4_high"),
            ("gpt-4".to_string(), " High".to_string())
        );
    }

    #[test]
    fn test_split_quality_suffix_medium() {
        assert_eq!(
            split_quality_suffix("model-medium"),
            ("model".to_string(), " Medium".to_string())
        );
    }

    #[test]
    fn test_split_quality_suffix_low() {
        assert_eq!(
            split_quality_suffix("model-low"),
            ("model".to_string(), " Low".to_string())
        );
    }

    #[test]
    fn test_split_quality_suffix_none() {
        assert_eq!(
            split_quality_suffix("model"),
            ("model".to_string(), String::new())
        );
        assert_eq!(
            split_quality_suffix("gpt-4"),
            ("gpt-4".to_string(), String::new())
        );
    }

    // ========== format_model_name tests ==========

    #[test]
    fn test_format_model_name_claude() {
        assert_eq!(
            format_model_name("claude-sonnet-4-20250514"),
            "Claude Sonnet 4"
        );
        assert_eq!(
            format_model_name("claude-3-5-sonnet-20241022"),
            "Claude 3.5 Sonnet"
        );
        assert_eq!(format_model_name("claude-3-opus-20240229"), "Claude 3 Opus");
    }

    #[test]
    fn test_format_model_name_gpt() {
        assert_eq!(format_model_name("gpt-4o"), "GPT-4o");
        assert_eq!(format_model_name("gpt-4o-mini"), "GPT-4o Mini");
        assert_eq!(format_model_name("gpt-5"), "GPT-5");
    }

    #[test]
    fn test_format_model_name_gemini() {
        assert_eq!(format_model_name("gemini-2.5-pro"), "Gemini 2.5 Pro");
        assert_eq!(format_model_name("gemini-1.5-flash"), "Gemini 1.5 Flash");
    }

    #[test]
    fn test_format_model_name_with_quality_suffix() {
        assert_eq!(format_model_name("gpt-4-high"), "GPT-4 High");
        assert_eq!(format_model_name("claude-opus-low"), "Claude opus Low");
    }

    // ========== get_provider_from_model tests ==========

    #[test]
    fn test_get_provider_from_model_anthropic() {
        assert_eq!(get_provider_from_model("claude-3-opus"), Some("anthropic"));
        assert_eq!(get_provider_from_model("sonnet-4"), Some("anthropic"));
        assert_eq!(get_provider_from_model("haiku-3.5"), Some("anthropic"));
    }

    #[test]
    fn test_get_provider_from_model_openai() {
        assert_eq!(get_provider_from_model("gpt-4"), Some("openai"));
        assert_eq!(get_provider_from_model("o1-preview"), Some("openai"));
        assert_eq!(get_provider_from_model("codex-001"), Some("openai"));
    }

    #[test]
    fn test_get_provider_from_model_google() {
        assert_eq!(get_provider_from_model("gemini-pro"), Some("google"));
        assert_eq!(get_provider_from_model("gemini-2.5-flash"), Some("google"));
    }

    #[test]
    fn test_get_provider_from_model_xai() {
        assert_eq!(get_provider_from_model("grok-3"), Some("xai"));
        assert_eq!(get_provider_from_model("grok-code"), Some("xai"));
    }

    #[test]
    fn test_get_provider_from_model_zai() {
        assert_eq!(get_provider_from_model("glm-4.7"), Some("zai"));
        assert_eq!(get_provider_from_model("pickle-model"), Some("zai"));
    }

    #[test]
    fn test_get_provider_from_model_unknown() {
        assert_eq!(get_provider_from_model("unknown-model"), None);
        assert_eq!(get_provider_from_model("random-123"), None);
    }

    // ========== calculate_intensity tests ==========

    #[test]
    fn test_calculate_intensity_grade4() {
        assert_eq!(calculate_intensity(100.0, 100.0), 4);
        assert_eq!(calculate_intensity(75.0, 100.0), 4);
        assert_eq!(calculate_intensity(80.0, 100.0), 4);
    }

    #[test]
    fn test_calculate_intensity_grade3() {
        assert_eq!(calculate_intensity(50.0, 100.0), 3);
        assert_eq!(calculate_intensity(60.0, 100.0), 3);
        assert_eq!(calculate_intensity(74.9, 100.0), 3);
    }

    #[test]
    fn test_calculate_intensity_grade2() {
        assert_eq!(calculate_intensity(25.0, 100.0), 2);
        assert_eq!(calculate_intensity(30.0, 100.0), 2);
        assert_eq!(calculate_intensity(49.9, 100.0), 2);
    }

    #[test]
    fn test_calculate_intensity_grade1() {
        assert_eq!(calculate_intensity(10.0, 100.0), 1);
        assert_eq!(calculate_intensity(24.9, 100.0), 1);
        assert_eq!(calculate_intensity(0.1, 100.0), 1);
    }

    #[test]
    fn test_calculate_intensity_grade0() {
        assert_eq!(calculate_intensity(0.0, 100.0), 0);
        assert_eq!(calculate_intensity(0.0, 0.0), 0);
    }

    // ========== calculate_streaks tests ==========

    #[test]
    fn test_calculate_streaks_consecutive() {
        let dates = vec![
            "2024-01-01".to_string(),
            "2024-01-02".to_string(),
            "2024-01-03".to_string(),
            "2024-01-04".to_string(),
        ];
        let (_current, longest) = calculate_streaks(&dates);
        assert_eq!(longest, 4);
    }

    #[test]
    fn test_calculate_streaks_with_gaps() {
        let dates = vec![
            "2024-01-01".to_string(),
            "2024-01-02".to_string(),
            "2024-01-05".to_string(),
            "2024-01-06".to_string(),
            "2024-01-07".to_string(),
        ];
        let (_current, longest) = calculate_streaks(&dates);
        assert_eq!(longest, 3);
    }

    #[test]
    fn test_calculate_streaks_empty() {
        let dates: Vec<String> = vec![];
        let (current, longest) = calculate_streaks(&dates);
        assert_eq!(current, 0);
        assert_eq!(longest, 0);
    }

    #[test]
    fn test_calculate_streaks_single_day() {
        let dates = vec!["2024-01-01".to_string()];
        let (_current, longest) = calculate_streaks(&dates);
        assert_eq!(longest, 1);
    }

    #[test]
    fn test_calculate_streaks_current_uses_provided_today() {
        let dates = vec![
            "2026-03-01".to_string(),
            "2026-03-02".to_string(),
            "2026-03-03".to_string(),
        ];
        let (current, longest) = calculate_streaks_with_today(&dates, "2026-03-03");
        assert_eq!(current, 3);
        assert_eq!(longest, 3);
    }

    // ========== date_diff_days tests ==========

    #[test]
    fn test_date_diff_days_forward() {
        assert_eq!(date_diff_days("2024-01-01", "2024-01-10"), 9);
        assert_eq!(date_diff_days("2024-01-01", "2024-01-02"), 1);
    }

    #[test]
    fn test_date_diff_days_backward() {
        assert_eq!(date_diff_days("2024-01-10", "2024-01-01"), 9);
        assert_eq!(date_diff_days("2024-01-02", "2024-01-01"), 1);
    }

    #[test]
    fn test_date_diff_days_same_day() {
        assert_eq!(date_diff_days("2024-01-01", "2024-01-01"), 0);
    }

    #[test]
    fn test_date_diff_days_invalid() {
        assert_eq!(date_diff_days("invalid", "2024-01-01"), 0);
        assert_eq!(date_diff_days("2024-01-01", "invalid"), 0);
        assert_eq!(date_diff_days("invalid", "invalid"), 0);
    }

    #[test]
    fn test_date_diff_days_cross_month() {
        assert_eq!(date_diff_days("2024-01-31", "2024-02-01"), 1);
        assert_eq!(date_diff_days("2024-01-01", "2024-02-01"), 31);
    }

    // ========== client_display_name tests ==========

    #[test]
    fn test_client_display_name_opencode() {
        assert_eq!(client_display_name("opencode"), Some("OpenCode"));
    }

    #[test]
    fn test_client_display_name_claude() {
        assert_eq!(client_display_name("claude"), Some("Claude Code"));
    }

    #[test]
    fn test_client_display_name_codex() {
        assert_eq!(client_display_name("codex"), Some("Codex CLI"));
    }

    #[test]
    fn test_client_display_name_gemini() {
        assert_eq!(client_display_name("gemini"), Some("Gemini CLI"));
    }

    #[test]
    fn test_client_display_name_cursor() {
        assert_eq!(client_display_name("cursor"), Some("Cursor IDE"));
    }

    #[test]
    fn test_client_display_name_amp() {
        assert_eq!(client_display_name("amp"), Some("Amp"));
    }

    #[test]
    fn test_client_display_name_droid() {
        assert_eq!(client_display_name("droid"), Some("Droid"));
    }

    #[test]
    fn test_client_display_name_openclaw() {
        assert_eq!(client_display_name("openclaw"), Some("OpenClaw"));
    }

    #[test]
    fn test_client_display_name_pi() {
        assert_eq!(client_display_name("pi"), Some("Pi"));
    }

    #[test]
    fn test_client_display_name_unknown() {
        assert_eq!(client_display_name("unknown"), None);
        assert_eq!(client_display_name(""), None);
        assert_eq!(client_display_name("Claude"), None); // case-sensitive
    }

    // ========== client_logo_url tests ==========

    #[test]
    fn test_client_logo_url_opencode() {
        assert_eq!(
            client_logo_url("OpenCode"),
            Some("https://tokscale.ai/assets/logos/opencode.png")
        );
    }

    #[test]
    fn test_client_logo_url_claude_code() {
        assert_eq!(
            client_logo_url("Claude Code"),
            Some("https://tokscale.ai/assets/logos/claude.jpg")
        );
    }

    #[test]
    fn test_client_logo_url_codex_cli() {
        assert_eq!(
            client_logo_url("Codex CLI"),
            Some("https://tokscale.ai/assets/logos/openai.jpg")
        );
    }

    #[test]
    fn test_client_logo_url_gemini_cli() {
        assert_eq!(
            client_logo_url("Gemini CLI"),
            Some("https://tokscale.ai/assets/logos/gemini.png")
        );
    }

    #[test]
    fn test_client_logo_url_cursor_ide() {
        assert_eq!(
            client_logo_url("Cursor IDE"),
            Some("https://tokscale.ai/assets/logos/cursor.jpg")
        );
    }

    #[test]
    fn test_client_logo_url_amp() {
        assert_eq!(
            client_logo_url("Amp"),
            Some("https://tokscale.ai/assets/logos/amp.png")
        );
    }

    #[test]
    fn test_client_logo_url_droid() {
        assert_eq!(
            client_logo_url("Droid"),
            Some("https://tokscale.ai/assets/logos/droid.png")
        );
    }

    #[test]
    fn test_client_logo_url_openclaw() {
        assert_eq!(
            client_logo_url("OpenClaw"),
            Some("https://tokscale.ai/assets/logos/openclaw.png")
        );
    }

    #[test]
    fn test_client_logo_url_pi() {
        assert_eq!(
            client_logo_url("Pi"),
            Some("https://tokscale.ai/assets/logos/pi.png")
        );
    }

    #[test]
    fn test_client_logo_url_unknown() {
        assert_eq!(client_logo_url("Unknown"), None);
        assert_eq!(client_logo_url(""), None);
        assert_eq!(client_logo_url("opencode"), None); // case-sensitive: expects display name
    }

    // ========== provider_logo_url tests ==========

    #[test]
    fn test_provider_logo_url_anthropic() {
        assert_eq!(
            provider_logo_url("anthropic"),
            Some("https://tokscale.ai/assets/logos/claude.jpg")
        );
    }

    #[test]
    fn test_provider_logo_url_openai() {
        assert_eq!(
            provider_logo_url("openai"),
            Some("https://tokscale.ai/assets/logos/openai.jpg")
        );
    }

    #[test]
    fn test_provider_logo_url_google() {
        assert_eq!(
            provider_logo_url("google"),
            Some("https://tokscale.ai/assets/logos/gemini.png")
        );
    }

    #[test]
    fn test_provider_logo_url_xai() {
        assert_eq!(
            provider_logo_url("xai"),
            Some("https://tokscale.ai/assets/logos/grok.jpg")
        );
    }

    #[test]
    fn test_provider_logo_url_zai() {
        assert_eq!(
            provider_logo_url("zai"),
            Some("https://tokscale.ai/assets/logos/zai.jpg")
        );
    }

    #[test]
    fn test_provider_logo_url_unknown() {
        assert_eq!(provider_logo_url("unknown"), None);
        assert_eq!(provider_logo_url(""), None);
        assert_eq!(provider_logo_url("Anthropic"), None); // case-sensitive
    }

    // ========== capitalize_word edge case tests ==========

    #[test]
    fn test_capitalize_word_unicode() {
        // Unicode chars that have uppercase variants
        assert_eq!(capitalize_word("über"), "Über");
        assert_eq!(capitalize_word("état"), "État");
    }

    #[test]
    fn test_capitalize_word_numbers() {
        assert_eq!(capitalize_word("123abc"), "123abc");
        assert_eq!(capitalize_word("42"), "42");
    }

    #[test]
    fn test_capitalize_word_with_hyphens() {
        // capitalize_word only handles a single word, hyphens stay
        assert_eq!(capitalize_word("hello-world"), "Hello-world");
    }

    #[test]
    fn test_capitalize_word_all_lowercase_long() {
        assert_eq!(capitalize_word("abcdefghij"), "Abcdefghij");
    }

    // ========== format_number_with_commas_i64 edge case tests ==========

    #[test]
    fn test_format_number_with_commas_i64_single_digit() {
        assert_eq!(format_number_with_commas_i64(1), "1");
        assert_eq!(format_number_with_commas_i64(9), "9");
    }

    #[test]
    fn test_format_number_with_commas_i64_exact_thousands() {
        assert_eq!(format_number_with_commas_i64(1_000), "1,000");
        assert_eq!(format_number_with_commas_i64(1_000_000), "1,000,000");
    }

    #[test]
    fn test_format_number_with_commas_i64_large_negative() {
        assert_eq!(
            format_number_with_commas_i64(-1_000_000_000),
            "-1,000,000,000"
        );
    }

    #[test]
    fn test_format_number_with_commas_i64_max_value() {
        // i64::MAX = 9_223_372_036_854_775_807
        let result = format_number_with_commas_i64(i64::MAX);
        assert_eq!(result, "9,223,372,036,854,775,807");
    }

    #[test]
    fn test_format_number_with_commas_i64_large_negative_max() {
        let result = format_number_with_commas_i64(-9_223_372_036_854_775_807);
        assert_eq!(result, "-9,223,372,036,854,775,807");
    }

    #[test]
    fn test_format_number_with_commas_i64_boundary_999() {
        assert_eq!(format_number_with_commas_i64(999), "999");
        assert_eq!(format_number_with_commas_i64(1000), "1,000");
    }

    // ========== format_cost edge case tests ==========

    #[test]
    fn test_format_cost_very_large() {
        assert_eq!(format_cost(1_000_000.0), "$1000.00K");
        assert_eq!(format_cost(50_000.0), "$50.00K");
    }

    #[test]
    fn test_format_cost_very_small() {
        assert_eq!(format_cost(0.001), "$0.00");
        assert_eq!(format_cost(0.005), "$0.01"); // rounds up
        assert_eq!(format_cost(0.01), "$0.01");
    }

    #[test]
    fn test_format_cost_negative() {
        // Negative costs produce negative dollar string
        assert_eq!(format_cost(-5.50), "$-5.50");
    }

    #[test]
    fn test_format_cost_boundary_at_1000() {
        assert_eq!(format_cost(999.99), "$999.99");
        assert_eq!(format_cost(1000.0), "$1.00K");
        assert_eq!(format_cost(1000.01), "$1.00K");
    }

    #[test]
    fn test_format_cost_fractional_thousands() {
        assert_eq!(format_cost(1500.0), "$1.50K");
        assert_eq!(format_cost(2999.99), "$3.00K");
    }

    // ========== get_provider_from_model edge case tests ==========

    #[test]
    fn test_get_provider_from_model_case_insensitive() {
        assert_eq!(get_provider_from_model("CLAUDE-3-OPUS"), Some("anthropic"));
        assert_eq!(get_provider_from_model("GPT-4"), Some("openai"));
        assert_eq!(get_provider_from_model("Gemini-Pro"), Some("google"));
        assert_eq!(get_provider_from_model("GROK-3"), Some("xai"));
        assert_eq!(get_provider_from_model("GLM-4.7"), Some("zai"));
    }

    #[test]
    fn test_get_provider_from_model_partial_matches() {
        // "opus" alone triggers anthropic
        assert_eq!(get_provider_from_model("opus-4"), Some("anthropic"));
        // "o1" triggers openai
        assert_eq!(get_provider_from_model("o1-mini"), Some("openai"));
        // "o3" triggers openai
        assert_eq!(get_provider_from_model("o3-mini"), Some("openai"));
    }

    #[test]
    fn test_get_provider_from_model_model_with_provider_prefix() {
        assert_eq!(
            get_provider_from_model("anthropic/claude-sonnet-4"),
            Some("anthropic")
        );
        assert_eq!(get_provider_from_model("openai/gpt-4o"), Some("openai"));
    }

    #[test]
    fn test_get_provider_from_model_empty_string() {
        assert_eq!(get_provider_from_model(""), None);
    }

    #[test]
    fn test_get_provider_from_model_pickle() {
        // "pickle" maps to zai
        assert_eq!(get_provider_from_model("big-pickle"), Some("zai"));
        assert_eq!(get_provider_from_model("pickle-3"), Some("zai"));
    }

    // ========== calculate_intensity edge case tests ==========

    #[test]
    fn test_calculate_intensity_zero_max_cost() {
        assert_eq!(calculate_intensity(50.0, 0.0), 0);
    }

    #[test]
    fn test_calculate_intensity_zero_cost() {
        assert_eq!(calculate_intensity(0.0, 100.0), 0);
    }

    #[test]
    fn test_calculate_intensity_both_zero() {
        assert_eq!(calculate_intensity(0.0, 0.0), 0);
    }

    #[test]
    fn test_calculate_intensity_equal_cost_max() {
        // ratio = 1.0, which is >= 0.75 → intensity 4
        assert_eq!(calculate_intensity(100.0, 100.0), 4);
    }

    #[test]
    fn test_calculate_intensity_exact_boundary_075() {
        assert_eq!(calculate_intensity(75.0, 100.0), 4);
    }

    #[test]
    fn test_calculate_intensity_exact_boundary_050() {
        assert_eq!(calculate_intensity(50.0, 100.0), 3);
    }

    #[test]
    fn test_calculate_intensity_exact_boundary_025() {
        assert_eq!(calculate_intensity(25.0, 100.0), 2);
    }

    #[test]
    fn test_calculate_intensity_just_below_025() {
        assert_eq!(calculate_intensity(24.99, 100.0), 1);
    }

    #[test]
    fn test_calculate_intensity_cost_exceeds_max() {
        // ratio > 1.0, still >= 0.75 → intensity 4
        assert_eq!(calculate_intensity(200.0, 100.0), 4);
    }

    #[test]
    fn test_calculate_intensity_tiny_fraction() {
        assert_eq!(calculate_intensity(0.001, 100.0), 1);
    }

    // ========== date_diff_days edge case tests ==========

    #[test]
    fn test_date_diff_days_cross_year() {
        assert_eq!(date_diff_days("2023-12-31", "2024-01-01"), 1);
        assert_eq!(date_diff_days("2023-01-01", "2024-01-01"), 365);
    }

    #[test]
    fn test_date_diff_days_leap_year() {
        // 2024 is a leap year
        assert_eq!(date_diff_days("2024-02-28", "2024-02-29"), 1);
        assert_eq!(date_diff_days("2024-02-28", "2024-03-01"), 2);
    }

    #[test]
    fn test_date_diff_days_large_gap() {
        assert_eq!(date_diff_days("2020-01-01", "2025-01-01"), 1827);
    }

    #[test]
    fn test_date_diff_days_partial_invalid() {
        assert_eq!(date_diff_days("2024-13-01", "2024-01-01"), 0); // month 13 invalid
        assert_eq!(date_diff_days("2024-01-01", "not-a-date"), 0);
    }

    #[test]
    fn test_date_diff_days_empty_strings() {
        assert_eq!(date_diff_days("", ""), 0);
        assert_eq!(date_diff_days("", "2024-01-01"), 0);
    }

    // ========== calculate_streaks comprehensive tests ==========

    #[test]
    fn test_calculate_streaks_no_consecutive_dates() {
        let dates = vec![
            "2024-01-01".to_string(),
            "2024-01-03".to_string(),
            "2024-01-05".to_string(),
            "2024-01-07".to_string(),
        ];
        let (_current, longest) = calculate_streaks(&dates);
        assert_eq!(longest, 1); // each date is isolated
    }

    #[test]
    fn test_calculate_streaks_multiple_separate_streaks() {
        let dates = vec![
            "2024-01-01".to_string(),
            "2024-01-02".to_string(),
            "2024-01-03".to_string(), // streak of 3
            "2024-01-10".to_string(),
            "2024-01-11".to_string(),
            "2024-01-12".to_string(),
            "2024-01-13".to_string(),
            "2024-01-14".to_string(), // streak of 5 — longest
            "2024-01-20".to_string(),
            "2024-01-21".to_string(), // streak of 2
        ];
        let (_current, longest) = calculate_streaks(&dates);
        assert_eq!(longest, 5);
    }

    #[test]
    fn test_calculate_streaks_longest_at_beginning() {
        let dates = vec![
            "2024-01-01".to_string(),
            "2024-01-02".to_string(),
            "2024-01-03".to_string(),
            "2024-01-04".to_string(), // streak of 4
            "2024-01-10".to_string(),
            "2024-01-11".to_string(), // streak of 2
        ];
        let (_current, longest) = calculate_streaks(&dates);
        assert_eq!(longest, 4);
    }

    #[test]
    fn test_calculate_streaks_all_consecutive() {
        let dates = vec![
            "2024-01-01".to_string(),
            "2024-01-02".to_string(),
            "2024-01-03".to_string(),
            "2024-01-04".to_string(),
            "2024-01-05".to_string(),
            "2024-01-06".to_string(),
            "2024-01-07".to_string(),
        ];
        let (_current, longest) = calculate_streaks(&dates);
        assert_eq!(longest, 7);
    }

    #[test]
    fn test_calculate_streaks_two_dates_consecutive() {
        let dates = vec!["2024-06-15".to_string(), "2024-06-16".to_string()];
        let (_current, longest) = calculate_streaks(&dates);
        assert_eq!(longest, 2);
    }

    #[test]
    fn test_calculate_streaks_two_dates_gap() {
        let dates = vec!["2024-06-15".to_string(), "2024-06-20".to_string()];
        let (_current, longest) = calculate_streaks(&dates);
        assert_eq!(longest, 1);
    }

    #[test]
    fn test_calculate_streaks_cross_month_boundary() {
        let dates = vec![
            "2024-01-30".to_string(),
            "2024-01-31".to_string(),
            "2024-02-01".to_string(),
            "2024-02-02".to_string(),
        ];
        let (_current, longest) = calculate_streaks(&dates);
        assert_eq!(longest, 4);
    }
}
