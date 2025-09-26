#![warn(clippy::pedantic)]

use std::{fs, path::PathBuf};

use anyhow::{Context, anyhow};
use base64::{Engine, prelude::BASE64_STANDARD};
use build_html::{Html, HtmlContainer, HtmlElement, HtmlPage, HtmlTag};
use clap::Parser;
use clap_verbosity_flag::Verbosity;

use crate::{
    api::{LoginReq, LoginRes},
    asset_list::Validate,
};

mod api;
mod asset_list;

#[derive(Parser)]
struct Args {
    /// The URL of the Homebox server
    #[arg(long, short)]
    server: String,

    /// The username for the Homebox server
    #[arg(long, short)]
    username: String,

    /// The password for the Homebox server. It is discouraged to
    /// provide the password through the command line - by omitting it,
    /// it will be requested on execution.
    #[arg(long, short)]
    password: Option<String>,

    /// The assets to generate labels for. This can be given as an
    /// individual, a range (using -- to join the start and end
    /// elements), or a list of both, e.g. 000-000--000-010,000-015
    #[arg(index = 1)]
    assets: String,

    /// The file path to output the result to.
    #[arg(index = 2)]
    output_html: PathBuf,

    /// The width of the page, in millimeters
    #[arg(long, default_value_t = 210.0)]
    page_width_mm: f64,

    /// The height of the page, in millimeters
    #[arg(long, default_value_t = 297.0)]
    page_height_mm: f64,

    /// The margin at the top of the page before the first row, in
    /// millimeters
    #[arg(long, default_value_t = 10.0)]
    page_margin_top_mm: f64,

    /// The margin to the left of the page, before the first column, in
    /// millimeters
    #[arg(long, default_value_t = 5.0)]
    page_margin_left_mm: f64,

    /// The margin at the bottom of the page after the last row, in
    /// millimeters
    #[arg(long, default_value_t = 10.0)]
    page_margin_bottom_mm: f64,

    /// The margin to the right of the page, after the last column, in
    /// millimeters
    #[arg(long, default_value_t = 5.0)]
    page_margin_right_mm: f64,

    /// The number of rows in the grid
    #[arg(long, default_value_t = 13)]
    grid_rows: usize,

    /// The number of columns in the grid
    #[arg(long, default_value_t = 5)]
    grid_columns: usize,

    /// The spacing between each grid row, in millimeters
    #[arg(long, default_value_t = 0.0)]
    grid_row_spacing_mm: f64,

    /// The spacing between each grid column, in millimeters
    #[arg(long, default_value_t = 2.5)]
    grid_col_spacing_mm: f64,

    /// Skip the first n elements of the grid to make better use of
    /// partially used sheets
    #[arg(long, short = 'S', default_value_t = 0)]
    grid_skip: usize,

    #[command(flatten)]
    verbose: Verbosity,
}

fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    tracing_subscriber::fmt()
        .with_max_level(args.verbose)
        .init();

    let client = reqwest::blocking::Client::new();
    let base_url = format!("{}/api", args.server);
    tracing::debug!("Base API URL: {base_url}");

    if fs::exists(&args.output_html).context("Failed to check is output exists already")? {
        Err(anyhow!(
            "Cannot overwrite output file! Please delete it first or change output destination."
        ))?;
    }

    // 1. Authenticate
    if args.password.is_some() {
        tracing::warn!(
            "The password has been provided on the command line. Note that this is less secure then providing it when requested."
        );
    }
    let password = args
        .password
        .or_else(|| {
            tracing::debug!("Prompting for password...");
            rpassword::prompt_password("Enter Homebox Password: ").ok()
        })
        .context("Failed to get password")?;

    tracing::info!("Authenticating...");
    let LoginRes { token, .. } = client
        .post(format!("{base_url}/v1/users/login"))
        .form(&LoginReq {
            username: args.username,
            password,
            stay_logged_in: false,
        })
        .send()
        .context("Failed to authenticate")?
        .json::<LoginRes>()
        .context("Failed to parse authentication response")?;
    tracing::debug!("Token acquired: {token}");

    // 2. Get label images
    let list = asset_list::parse(args.assets).context("Failed to parse asset list")?;
    tracing::debug!("Assets: {list:?}");
    list.validate().context("Failed to validate asset list")?;

    let mut labels = vec![];
    for entry in list {
        for asset_id in entry {
            tracing::info!("Getting label for asset ID: {asset_id}");
            let label_bytes = client
                .get(format!(
                    "{base_url}/v1/labelmaker/asset/{asset_id}?print=false"
                ))
                .header("Authorization", &token)
                .send()
                .context("Failed to get asset label")?
                .error_for_status()
                .context("Failed to get asset label (are all the provided asset IDs valid?)")?
                .bytes()
                .context("Failed to parse image")?;
            labels.push(label_bytes);
        }
    }

    // 3. Build page(s)
    let num_per_page = args.grid_rows * args.grid_columns;
    tracing::info!(
        "Producing {} pages...",
        (args.grid_skip + labels.len()) / num_per_page + 1
    );

    let configurable_style = format!(
        r"
        .page {{
            --pad-top: {}mm;
            --pad-left: {}mm;
            --pad-bottom: {}mm;
            --pad-right: {}mm;
            width: calc({}mm - var(--pad-left) - var(--pad-right));
            height: calc({}mm - var(--pad-top) - var(--pad-bottom));
            padding-top: var(--pad-top);
            padding-left: var(--pad-left);
            padding-bottom: var(--pad-bottom);
            padding-right: var(--pad-right);
            grid-template-columns: repeat({}, 1fr);
            grid-template-rows: repeat({}, 1fr);
            row-gap: {}mm;
            column-gap: {}mm;
        }}
    ",
        args.page_margin_top_mm,
        args.page_margin_left_mm,
        args.page_margin_bottom_mm,
        args.page_margin_right_mm,
        args.page_width_mm,
        args.page_height_mm,
        args.grid_columns,
        args.grid_rows,
        args.grid_row_spacing_mm,
        args.grid_col_spacing_mm
    );

    let page = generate_html(num_per_page, configurable_style, args.grid_skip, &labels);
    fs::write(args.output_html, page.to_html_string()).context("Failed to write output")?;

    Ok(())
}

/// Generate the HTML itself
fn generate_html(
    num_per_page: usize,
    configurable_style: String,
    grid_skip: usize,
    labels: &[bytes::Bytes],
) -> HtmlPage {
    let mut page = HtmlPage::new()
        .with_title("Homebox Labels")
        .with_style(include_str!("style.css"))
        .with_style(configurable_style);

    page.add_paragraph_attr(include_str!("notice.txt"), [("class", "no-print")]);

    let mut skip_first = true;
    let mut page_div = HtmlElement::new(HtmlTag::Div).with_attribute("class", "page");
    for i in 0..grid_skip {
        // Create empty elems
        if i % num_per_page == 0 {
            // Create page div
            if skip_first {
                skip_first = false;
            } else {
                page.add_raw(page_div.to_html_string());
            }
            page_div = HtmlElement::new(HtmlTag::Div).with_attribute("class", "page");
        }
        page_div.add_child(HtmlElement::new(HtmlTag::Div).with_raw("").into());
    }
    for (idx, bytes) in labels.iter().enumerate() {
        let idx = idx + grid_skip;
        if idx % num_per_page == 0 {
            // Create page div
            if skip_first {
                skip_first = false;
            } else {
                page.add_raw(page_div.to_html_string());
            }
            page_div = HtmlElement::new(HtmlTag::Div).with_attribute("class", "page");
        }

        let data = BASE64_STANDARD.encode(bytes);
        page_div.add_child(
            HtmlElement::new(HtmlTag::Div)
                .with_attribute(
                    "style",
                    format!("background-image: url(data:image/png;base64,{data})"),
                )
                .with_raw("")
                .into(),
        );
    }
    page.add_raw(page_div.to_html_string());

    page
}
