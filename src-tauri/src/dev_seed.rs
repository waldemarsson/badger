//! Dev-only database seeding. Excluded from release builds via #[cfg(debug_assertions)].

#[cfg(debug_assertions)]
use chrono::{Datelike, Duration, Local, NaiveDate, Utc, Weekday};
#[cfg(debug_assertions)]
use sqlx::{Pool, Sqlite};
#[cfg(debug_assertions)]
use tauri::State;

#[cfg(debug_assertions)]
use crate::AppError;

#[cfg(debug_assertions)]
fn seed_start_date(today: NaiveDate) -> NaiveDate {
    let mut year = today.year();
    let mut month = today.month() as i32 - 2;

    while month <= 0 {
        month += 12;
        year -= 1;
    }

    NaiveDate::from_ymd_opt(year, month as u32, 1)
        .expect("seed start date must be a valid month boundary")
}

#[cfg(debug_assertions)]
async fn seed_time_entry(
    tx: &mut sqlx::Transaction<'_, Sqlite>,
    issue_id: i64,
    date: NaiveDate,
    hours: f64,
    notes: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query("INSERT INTO time_entries (IssueId, Date, Hours, Notes) VALUES (?, ?, ?, ?)")
        .bind(issue_id)
        .bind(date.format("%Y-%m-%d").to_string())
        .bind(hours)
        .bind(notes)
        .execute(&mut **tx)
        .await?;

    Ok(())
}

#[cfg(debug_assertions)]
async fn seed_trip(
    tx: &mut sqlx::Transaction<'_, Sqlite>,
    date: NaiveDate,
    from_place: &str,
    to_place: &str,
    km: Option<f64>,
    amount: Option<f64>,
) -> Result<(), sqlx::Error> {
    sqlx::query("INSERT INTO travel (Date, FromPlace, ToPlace, Km, Amount) VALUES (?, ?, ?, ?, ?)")
        .bind(date.format("%Y-%m-%d").to_string())
        .bind(from_place)
        .bind(to_place)
        .bind(km)
        .bind(amount)
        .execute(&mut **tx)
        .await?;

    Ok(())
}

#[cfg(debug_assertions)]
pub(crate) async fn seed_test_data_if_empty_with_anchor(
    pool: &Pool<Sqlite>,
    today: NaiveDate,
) -> Result<(), sqlx::Error> {
    let existing_customers: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM customers")
        .fetch_one(pool)
        .await?;

    if existing_customers.0 == 0 {
        seed_core_data(pool, today).await?;
    }
    seed_notes_if_empty(pool).await?;
    seed_reminders_if_empty(pool, crate::commands::reminders::local_now()).await
}

/// A few sample reminders around `now`: overdue, later today, tomorrow, recurring, one
/// linked to the "Weekly sync" note when it exists, and one done. Runs whenever the
/// `reminders` table is empty.
#[cfg(debug_assertions)]
async fn seed_reminders_if_empty(
    pool: &Pool<Sqlite>,
    now: chrono::NaiveDateTime,
) -> Result<(), sqlx::Error> {
    use crate::commands::reminders::{
        first_occurrence, format_time_of_day, format_timestamp, local_exists, Rule,
    };
    use chrono::NaiveTime;

    let existing: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM reminders")
        .fetch_one(pool)
        .await?;
    if existing.0 > 0 {
        return Ok(());
    }
    let note_id: Option<i64> = sqlx::query_scalar("SELECT id FROM notes WHERE Title = ?")
        .bind("Weekly sync")
        .fetch_optional(pool)
        .await?;

    let at = |t: chrono::NaiveDateTime| format_timestamp(t);
    let hm = |h, m| NaiveTime::from_hms_opt(h, m, 0).unwrap_or(NaiveTime::MIN);
    let tomorrow = now.date() + Duration::days(1);
    let standup = first_occurrence(&Rule::Weekdays, now, hm(9, 0), local_exists)
        .unwrap_or(tomorrow.and_time(hm(9, 0)));
    let rent = first_occurrence(&Rule::Monthly(25), now, hm(8, 0), local_exists)
        .unwrap_or(tomorrow.and_time(hm(8, 0)));
    let later = now + Duration::hours(3);
    let overdue = now - Duration::hours(2);
    let yesterday = now - Duration::days(1);

    // (text, remind_at, time_of_day, repeat, weekdays, month_day, state, note_id)
    type Row<'a> = (
        &'a str,
        String,
        String,
        &'a str,
        &'a str,
        Option<i64>,
        &'a str,
        Option<i64>,
    );
    let rows: [Row; 6] = [
        (
            "Send invoice to Northwind",
            at(overdue),
            format_time_of_day(overdue.time()),
            "none",
            "",
            None,
            "fired",
            None,
        ),
        (
            "Prepare the weekly sync",
            at(later),
            format_time_of_day(later.time()),
            "none",
            "",
            None,
            "pending",
            note_id,
        ),
        (
            "Call Anna about the release",
            at(tomorrow.and_time(hm(14, 0))),
            "14:00".into(),
            "none",
            "",
            None,
            "pending",
            None,
        ),
        (
            "Standup",
            at(standup),
            "09:00".into(),
            "weekdays",
            "",
            None,
            "pending",
            None,
        ),
        (
            "Pay rent",
            at(rent),
            "08:00".into(),
            "monthly",
            "",
            Some(25),
            "pending",
            None,
        ),
        (
            "Book the dentist",
            at(yesterday),
            format_time_of_day(yesterday.time()),
            "none",
            "",
            None,
            "done",
            None,
        ),
    ];
    let stamp = at(now);
    let mut tx = pool.begin().await?;
    for (text, remind_at, time_of_day, repeat, weekdays, month_day, state, note_id) in rows {
        sqlx::query(
            "INSERT INTO reminders (Text, RemindAt, TimeOfDay, Repeat, Weekdays, MonthDay, State, \
                 NoteId, CreatedAt, UpdatedAt) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(text)
        .bind(remind_at)
        .bind(time_of_day)
        .bind(repeat)
        .bind(weekdays)
        .bind(month_day)
        .bind(state)
        .bind(note_id)
        .bind(&stamp)
        .bind(&stamp)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await
}

/// Customers, projects, issues, time entries, and trips.
#[cfg(debug_assertions)]
async fn seed_core_data(pool: &Pool<Sqlite>, today: NaiveDate) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;

    let northwind_customer_id: i64 =
        sqlx::query_scalar("INSERT INTO customers (Name, UseAsDefault) VALUES (?, ?) RETURNING id")
            .bind("Northwind Studio")
            .bind(1_i64)
            .fetch_one(&mut *tx)
            .await?;

    let blue_harbor_customer_id: i64 =
        sqlx::query_scalar("INSERT INTO customers (Name, UseAsDefault) VALUES (?, ?) RETURNING id")
            .bind("Blue Harbor Labs")
            .bind(0_i64)
            .fetch_one(&mut *tx)
            .await?;

    let desktop_project_id: i64 = sqlx::query_scalar(
        "INSERT INTO projects (CustomerId, Name, UseAsDefault) VALUES (?, ?, ?) RETURNING id",
    )
    .bind(northwind_customer_id)
    .bind("Badger Desktop")
    .bind(1_i64)
    .fetch_one(&mut *tx)
    .await?;

    let marketing_project_id: i64 = sqlx::query_scalar(
        "INSERT INTO projects (CustomerId, Name, UseAsDefault) VALUES (?, ?, ?) RETURNING id",
    )
    .bind(northwind_customer_id)
    .bind("Northwind Marketing Site")
    .bind(0_i64)
    .fetch_one(&mut *tx)
    .await?;

    let operations_project_id: i64 = sqlx::query_scalar(
        "INSERT INTO projects (CustomerId, Name, UseAsDefault) VALUES (?, ?, ?) RETURNING id",
    )
    .bind(blue_harbor_customer_id)
    .bind("Operations Dashboard")
    .bind(1_i64)
    .fetch_one(&mut *tx)
    .await?;

    let planning_issue_id: i64 =
        sqlx::query_scalar("INSERT INTO issues (ProjectId, Title) VALUES (?, ?) RETURNING id")
            .bind(desktop_project_id)
            .bind("Backlog triage and sprint planning")
            .fetch_one(&mut *tx)
            .await?;

    let reporting_issue_id: i64 =
        sqlx::query_scalar("INSERT INTO issues (ProjectId, Title) VALUES (?, ?) RETURNING id")
            .bind(desktop_project_id)
            .bind("Weekly and monthly reporting improvements")
            .fetch_one(&mut *tx)
            .await?;

    let settings_issue_id: i64 =
        sqlx::query_scalar("INSERT INTO issues (ProjectId, Title) VALUES (?, ?) RETURNING id")
            .bind(desktop_project_id)
            .bind("Settings polish and QA fixes")
            .fetch_one(&mut *tx)
            .await?;

    let landing_issue_id: i64 =
        sqlx::query_scalar("INSERT INTO issues (ProjectId, Title) VALUES (?, ?) RETURNING id")
            .bind(marketing_project_id)
            .bind("Landing page refresh")
            .fetch_one(&mut *tx)
            .await?;

    let content_issue_id: i64 =
        sqlx::query_scalar("INSERT INTO issues (ProjectId, Title) VALUES (?, ?) RETURNING id")
            .bind(marketing_project_id)
            .bind("Content updates and analytics review")
            .fetch_one(&mut *tx)
            .await?;

    let admin_issue_id: i64 =
        sqlx::query_scalar("INSERT INTO issues (ProjectId, Title) VALUES (?, ?) RETURNING id")
            .bind(operations_project_id)
            .bind("Admin workflow cleanup")
            .fetch_one(&mut *tx)
            .await?;

    let export_issue_id: i64 =
        sqlx::query_scalar("INSERT INTO issues (ProjectId, Title) VALUES (?, ?) RETURNING id")
            .bind(operations_project_id)
            .bind("Billing export fixes")
            .fetch_one(&mut *tx)
            .await?;

    let start_date = seed_start_date(today);
    let total_days = (today - start_date).num_days();

    for day_offset in 0..=total_days {
        let date = start_date + Duration::days(day_offset);
        let week_index = day_offset / 7;

        match date.weekday() {
            Weekday::Mon => {
                seed_time_entry(
                    &mut tx,
                    planning_issue_id,
                    date,
                    2.5 + (week_index % 3) as f64 * 0.5,
                    "Sprint planning, backlog grooming, and team sync.",
                )
                .await?;
            }
            Weekday::Tue => {
                seed_time_entry(
                    &mut tx,
                    reporting_issue_id,
                    date,
                    3.5 + (week_index % 2) as f64 * 0.5,
                    "Worked through weekly report edge cases and review feedback.",
                )
                .await?;

                if week_index % 2 == 0 {
                    seed_time_entry(
                        &mut tx,
                        settings_issue_id,
                        date,
                        1.0,
                        "Polished settings UX and verified form behaviour.",
                    )
                    .await?;
                }
            }
            Weekday::Wed => {
                seed_time_entry(
                    &mut tx,
                    landing_issue_id,
                    date,
                    3.0 + (week_index % 3) as f64 * 0.5,
                    "Updated landing page sections, imagery, and copy layout.",
                )
                .await?;

                if week_index % 2 == 0 {
                    seed_trip(
                        &mut tx,
                        date,
                        "Malmö",
                        "Karlskrona",
                        Some(500.0),
                        Some(1300.0),
                    )
                    .await?;
                } else if week_index % 4 == 1 {
                    seed_trip(&mut tx, date, "Malmö", "Lund", Some(42.5), None).await?;
                }
            }
            Weekday::Thu => {
                seed_time_entry(
                    &mut tx,
                    admin_issue_id,
                    date,
                    4.0 + (week_index % 2) as f64 * 0.5,
                    "Refined admin workflow steps and handled follow-up fixes.",
                )
                .await?;

                if week_index % 3 == 1 {
                    seed_time_entry(
                        &mut tx,
                        content_issue_id,
                        date,
                        1.5,
                        "Reviewed content tweaks and analytics event coverage.",
                    )
                    .await?;
                }
            }
            Weekday::Fri => {
                seed_time_entry(
                    &mut tx,
                    export_issue_id,
                    date,
                    2.5 + (week_index % 4) as f64 * 0.5,
                    "Closed out export bugs, validation, and stakeholder notes.",
                )
                .await?;
            }
            Weekday::Sat | Weekday::Sun => {}
        }
    }

    tx.commit().await?;
    Ok(())
}

#[cfg(debug_assertions)]
const SEED_NOTE_WELCOME: &str = "# Welcome to Notes

Notes are **markdown** with a *live preview*: markers like `**` hide unless the cursor is on the line.

## Inline styles

**Bold**, *italic*, ~~strikethrough~~, `inline code`, and a [link](https://tauri.app) with its URL hidden.

> Blockquotes look like this.
> They can span lines.

---

### Lists

- Bullet one
- Bullet two
  - Nested bullet
1. First
2. Second

- [ ] Open task
- [x] Finished task

![Images stay as raw markdown](https://example.com/image.png)
";

#[cfg(debug_assertions)]
const SEED_NOTE_TABLES: &str = "Tables render as a grid until the cursor enters them.

| Task | Owner | Hours |
| :--- | :---: | ----: |
| **Release** notes | Åsa | 1.5 |
| `cargo test` run | Erik | 0.25 |
| ~~Old item~~ | *nobody* | 0 |
| Short row |

Rows with too few cells are padded.
";

#[cfg(debug_assertions)]
const SEED_NOTE_CODE: &str = "Setext heading
==============

Smaller setext heading
----------------------

```rust
fn main() {
    println!(\"Hello from a fenced block\");
}
```

```bash
npm run vite:build
```
";

#[cfg(debug_assertions)]
const SEED_NOTE_MEETING: &str = "## Weekly sync

- Demo the *Notes* page
- [ ] Follow up on billing export
- [x] Ship the Today overview

> Decision: keep notes local-first.
";

#[cfg(debug_assertions)]
const SEED_NOTE_ARCHIVED: &str = "# Old roadmap

Lives in **Projects / Archive**, a nested folder.
";

/// Creates `Projects / Archive` when there are no folders yet; otherwise reuses an existing
/// seeded `Archive` (so reseeding after deleting every note doesn't duplicate folders).
/// Returns the Archive folder, or `None` (seed at the root) when other folders exist.
#[cfg(debug_assertions)]
async fn seed_folders(tx: &mut sqlx::Transaction<'_, Sqlite>) -> Result<Option<i64>, sqlx::Error> {
    let existing_folders: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM folders")
        .fetch_one(&mut **tx)
        .await?;
    if existing_folders.0 > 0 {
        return sqlx::query_scalar(
            "SELECT c.id FROM folders c JOIN folders p ON p.id = c.ParentId \
             WHERE c.Name = 'Archive' AND p.Name = 'Projects' AND p.ParentId IS NULL \
             ORDER BY c.id LIMIT 1",
        )
        .fetch_optional(&mut **tx)
        .await;
    }

    let projects_folder_id: i64 = sqlx::query_scalar(
        "INSERT INTO folders (ParentId, Name, Position) VALUES (NULL, ?, 0) RETURNING id",
    )
    .bind("Projects")
    .fetch_one(&mut **tx)
    .await?;
    let archive_folder_id: i64 = sqlx::query_scalar(
        "INSERT INTO folders (ParentId, Name, Position) VALUES (?, ?, 0) RETURNING id",
    )
    .bind(projects_folder_id)
    .bind("Archive")
    .fetch_one(&mut **tx)
    .await?;
    Ok(Some(archive_folder_id))
}

/// Sample notes covering every live-preview element, plus a folder with a nested child.
/// Runs whenever the `notes` table is empty, independent of the customer seed.
#[cfg(debug_assertions)]
async fn seed_notes_if_empty(pool: &Pool<Sqlite>) -> Result<(), sqlx::Error> {
    let existing_notes: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM notes")
        .fetch_one(pool)
        .await?;
    if existing_notes.0 > 0 {
        return Ok(());
    }

    let mut tx = pool.begin().await?;
    let archive_folder_id = seed_folders(&mut tx).await?;

    let mut long_note =
        String::from("# Long note\n\nScroll down: the table at the end still renders.\n\n");
    for i in 1..=300 {
        long_note.push_str(&format!("Line {i} of filler text.\n"));
    }
    long_note.push_str("\n| Far | Down |\n| --- | --- |\n| still | rendered |\n");

    let now = Local::now();
    let notes: [(Option<i64>, &str, &str, bool); 6] = [
        (None, "Welcome to Notes", SEED_NOTE_WELCOME, true),
        (None, "Markdown tables", SEED_NOTE_TABLES, false),
        (None, "Code and setext headings", SEED_NOTE_CODE, false),
        (None, "Weekly sync", SEED_NOTE_MEETING, false),
        (None, "Long note", &long_note, false),
        (archive_folder_id, "Old roadmap", SEED_NOTE_ARCHIVED, false),
    ];
    for (age_hours, (folder_id, title, body, pinned)) in (0_i64..).zip(notes) {
        let ts = (now - Duration::hours(age_hours * 7))
            .format("%Y-%m-%dT%H:%M:%S%.3f")
            .to_string();
        sqlx::query(
            "INSERT INTO notes (FolderId, Title, Body, Pinned, CreatedAt, UpdatedAt) \
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(folder_id)
        .bind(title)
        .bind(body)
        .bind(pinned)
        .bind(&ts)
        .bind(&ts)
        .execute(&mut *tx)
        .await?;
    }
    crate::commands::notes::rebuild_index(&mut tx).await?;

    tx.commit().await?;
    Ok(())
}

/// Bilingual sample notes for trying semantic search: Swedish/English pairs that cover the same
/// topic in different words (budget, release date, onboarding, invoice dispute, backups,
/// accessibility, summer leave), plus two unrelated decoys. Each body is 100–200 words.
#[cfg(debug_assertions)]
const SAMPLE_NOTES: [(&str, &str); 16] = [
    (
        "Budget forecast for next year",
        r##"# Budget forecast for next year

#blue-harbor-labs asked for a cost outlook before the board meeting in December.

## Where the money goes

- Hosting has grown about 12% per quarter since the dashboard launched.
- Contractor hours are the largest line item and the hardest to predict.
- Licences for the design tools renew in March, so they land in Q1.

## Assumptions

We expect the same team size, no new office, and a modest increase in travel once the second site opens. If the client adds the reporting module we discussed, development spend rises by roughly a third in the first half of the year.

## Open questions

- [ ] Should we pre-pay hosting for twelve months to get the discount?
- [ ] Who signs off on contractor overruns above 10%?
- [ ] Do we split the forecast per quarter or per project?

Send a draft to finance by Friday so they can merge it with the other departments before the review.
"##,
    ),
    (
        "Ekonomimöte om prognosen",
        r##"# Ekonomimöte om prognosen

Ekonomichefen vill se en prognos per kvartal i stället för en siffra för hela året. Resekostnaderna sticker ut och behöver förklaras bättre.

## Beslut

- Vi delar upp kalkylen per kvartal och per kund.
- Driftkostnaden för servrarna redovisas separat från utvecklingstimmarna.
- Konsulternas timmar får inte överskrida ramen med mer än tio procent utan godkännande.

## Diskussion

Det fanns oro för att licenserna förnyas samtidigt som det nya kontoret öppnar, vilket ger en topp i första kvartalet. Någon föreslog att betala serveravtalet för ett helt år i förväg för att få rabatt, men det binder kapital.

## Att göra

- [ ] Ta fram en ny kalkyl med tre scenarier: låg, normal och hög.
- [ ] Fråga leverantören om rabatten gäller även vid kvartalsvis betalning.
- [ ] Boka uppföljning med ledningsgruppen om två veckor.
"##,
    ),
    (
        "Flytta lanseringsdatumet",
        r##"# Flytta lanseringsdatumet

Vi kommer inte hinna till den planerade releasen den femtonde. Integrationen mot betalsystemet är inte klar och testmiljön har varit nere i tre dagar.

## Förslag

Skjut lanseringen två veckor framåt och släpp en mindre version först, utan exportfunktionen. Kunden har sagt att de hellre får något stabilt än allt på en gång.

## Risker

- Marknadsavdelningen har redan bokat annonser för den gamla veckan.
- Supporten behöver utbildning innan de nya flödena går live.
- Om vi skjuter mer än två veckor krockar det med semestrarna.

## Nästa steg

- [ ] Informera kunden i dag och be om skriftligt godkännande.
- [ ] Uppdatera tidplanen i projektverktyget.
- [ ] Be marknad flytta kampanjen eller pausa den.
- [ ] Sätt upp en extra testmiljö så att vi inte blir stående igen.

Beslut tas på torsdagens styrgruppsmöte.
"##,
    ),
    (
        "Release slipping two weeks",
        r##"# Release slipping two weeks

The go-live we promised for mid-month is not realistic. The payment provider integration still fails in staging, and QA lost three days when the test server went down.

## What we tell the client

We ship a smaller first version on time, without the export feature, and deliver the rest a fortnight later. They told us last week that stability matters more than scope.

## Knock-on effects

- Ads were booked for the original week and may need to move.
- Support staff need a short training session before the new flows reach customers.
- Pushing further than two weeks runs straight into the summer holidays.

## Actions

- [ ] Email the client today and ask for written approval of the new date.
- [ ] Update the timeline in the project tracker.
- [ ] Ask marketing to pause the campaign.
- [ ] Add a second staging environment so one outage does not block everyone.
"##,
    ),
    (
        "Onboarding a new developer",
        r##"# Onboarding a new developer

A short checklist so the first week feels organised instead of chaotic.

## Before day one

- Order the laptop and make sure it arrives two days early.
- Create accounts for email, chat, the code host and the issue tracker.
- Pick a buddy who is not on call that week.

## First week

1. Walk through the architecture document together, not alone.
2. Pair on a small, real bug fix and ship it to production by Wednesday.
3. Read the incident write-ups from the last six months.
4. Join every stand-up and one customer call, just to listen.

## Things people usually forget

- Access to the shared password vault.
- Adding them to the on-call calendar, but not as primary yet.
- Explaining how we log time against projects and issues.

Check in on Friday: what was confusing, what was missing, and what should change in this list?
"##,
    ),
    (
        "Introduktion för nyanställda",
        r##"# Introduktion för nyanställda

Det här är upplägget när någon börjar i teamet, så att första veckan inte blir rörig.

## Innan första dagen

- Beställ dator i god tid och kontrollera att den levereras.
- Skapa konton för mejl, chatt, kodförrådet och ärendehanteringen.
- Utse en fadder som inte har jour samma vecka.

## Första veckan

1. Gå igenom systemets uppbyggnad tillsammans med faddern.
2. Rätta en liten bugg i par och driftsätt den före onsdag.
3. Läs igenom rapporterna från tidigare driftstörningar.
4. Var med på dagliga avstämningar och lyssna på ett kundsamtal.

## Sådant som brukar glömmas

- Behörighet till lösenordshanteraren.
- Att lägga in personen i jourschemat, men inte som första linje än.
- Hur vi rapporterar tid mot projekt och ärenden.

På fredagen pratar vi om vad som var otydligt och vad som borde ändras i listan.
"##,
    ),
    (
        "Kunden bestrider fakturan",
        r##"# Kunden bestrider fakturan

#northwind-studio hörde av sig och menar att oktoberfakturan innehåller timmar som aldrig godkändes. Det gäller främst arbetet med den nya startsidan.

## Vad vi vet

- Timmarna är loggade och kopplade till rätt ärenden.
- Ändringsbeställningen skickades per mejl men bekräftades aldrig skriftligt.
- Projektledaren på deras sida har bytts ut under hösten.

## Hur vi går vidare

Vi skickar en specifikation per ärende och datum, tillsammans med mejlet där tillägget beställdes. Om de fortfarande inte vill betala föreslår vi en kreditnota på halva beloppet för att bevara relationen.

## Lärdomar

- Ändringar ska alltid bekräftas skriftligt innan arbetet börjar.
- Skicka en månadsrapport med timmar så att ingen blir överraskad av fakturan.

- [ ] Ta fram specifikationen senast måndag.
- [ ] Stäm av med ekonomi innan vi lovar någon kreditering.
"##,
    ),
    (
        "Invoice dispute follow-up",
        r##"# Invoice dispute follow-up

The client pushed back on last month's bill, saying several hours were never approved. Most of the disputed time is the homepage redesign.

## Facts

- Every hour is logged against the right issue.
- The change request went out by email, but nobody on their side confirmed it.
- Their project manager changed in September and the new one was not briefed.

## Plan

Send an itemised breakdown per issue and date, with the original email attached. If they still refuse, offer a credit note for half the disputed amount to protect the relationship; the account is worth far more than this month.

## Process changes

- Get written confirmation before starting any change request.
- Send a monthly hours summary so the bill is never a surprise.

- [ ] Prepare the breakdown by Monday.
- [ ] Check with finance before promising any credit.
"##,
    ),
    (
        "Backup strategy",
        r##"# Backup strategy

How we keep customer data safe, and how we prove that we can get it back.

## Schedule

- Nightly full database dump, kept for thirty days.
- Hourly incremental snapshots, kept for two days.
- A weekly copy goes to a different cloud region.

## The part people skip

Backups are worthless until someone has restored one. Once a month we pick a random snapshot, restore it into an isolated environment, and run the smoke tests against it. We write down how long it took, because that number is our real recovery time.

## Last drill

The restore worked, but it took ninety minutes instead of the thirty we promise in the contract. Most of the time went to rebuilding search indexes after the data was back.

- [ ] Rebuild indexes in parallel during restore.
- [ ] Alert when a nightly dump is smaller than the previous one by more than 20%.
- [ ] Document the restore steps so anyone on call can do it.
"##,
    ),
    (
        "Återläsningstest av säkerhetskopior",
        r##"# Återläsningstest av säkerhetskopior

En säkerhetskopia är bara värd något om vi faktiskt kan läsa tillbaka den. Därför övar vi varje månad.

## Så gick det till

Vi valde en slumpvis ögonblicksbild från förra veckan och läste in den i en isolerad miljö. Därefter körde vi de automatiska kontrollerna för att se att inloggning, rapporter och export fungerade.

## Resultat

Återställningen lyckades, men tog en och en halv timme. I avtalet utlovar vi en halvtimme. Det mesta av tiden gick åt till att bygga om sökindexen efter att datan var på plats.

## Förbättringar

- Bygg om indexen parallellt i stället för ett i taget.
- Larma om nattens dump är betydligt mindre än föregående natt.
- Skriv en tydlig instruktion så att den som har jour kan göra detta utan hjälp.
- Spara en kopia i en annan region varje vecka.

Nästa övning sker första måndagen i nästa månad.
"##,
    ),
    (
        "Accessibility review of the landing page",
        r##"# Accessibility review of the landing page

Findings from going through the new #northwind-studio/northwind-marketing-site pages with a screen reader and keyboard only.

## Problems

- Grey body text on the light background fails the contrast ratio for normal text.
- The hero carousel moves on its own and cannot be paused.
- Several icon buttons have no label, so the screen reader just says "button".
- Focus disappears after closing the newsletter dialog.

## Quick wins

1. Darken the body text one step in the palette.
2. Add visible focus outlines instead of removing them in the reset styles.
3. Give every icon button an accessible name.

## Bigger items

- Replace the carousel with a static hero, or add pause controls.
- Test forms with error messages read aloud.

We should add an automated check to the build so regressions are caught before review, and repeat the manual pass before each release.
"##,
    ),
    (
        "Tillgänglighet på webbplatsen",
        r##"# Tillgänglighet på webbplatsen

Vi gick igenom de nya sidorna med skärmläsare och enbart tangentbord. Här är vad vi hittade.

## Brister

- Den ljusgrå brödtexten har för låg kontrast mot bakgrunden.
- Bildspelet högst upp rör sig automatiskt och går inte att stoppa.
- Flera knappar består bara av en ikon och saknar beskrivning, så skärmläsaren säger bara "knapp".
- Markeringen försvinner när man stänger rutan för nyhetsbrevet.

## Enkla åtgärder

1. Gör brödtexten ett steg mörkare.
2. Visa tydliga fokusramar i stället för att ta bort dem.
3. Ge alla ikonknappar en beskrivande text.

## Större förändringar

- Byt bildspelet mot en fast bild, eller lägg till en pausknapp.
- Testa formulären så att felmeddelanden läses upp.

Vi lägger till en automatisk kontroll i bygget och gör en manuell genomgång före varje release.
"##,
    ),
    (
        "Semesterplanering i teamet",
        r##"# Semesterplanering i teamet

Sommaren närmar sig och vi behöver se till att någon alltid finns på plats för kunderna.

## Önskemål hittills

- Anna vill ha veckorna 28 till 31.
- Erik tar helst ledigt i juni och första veckan i juli.
- Sara är flexibel men vill ha minst tre veckor i följd.

## Regler

Minst två personer ska vara i tjänst varje vecka, och en av dem måste kunna hantera driftstörningar. Jouren roterar som vanligt men med längre pass, eftersom färre delar på den.

## Kundkontakt

Vi meddelar alla kunder i god tid vilka veckor svarstiderna blir längre. Akuta ärenden går till jourtelefonen.

- [ ] Lägg in allas önskemål i kalendern senast den sista april.
- [ ] Stäm av överlappningar på nästa teammöte.
- [ ] Skicka information till kunderna i maj.
"##,
    ),
    (
        "Summer leave and on-call cover",
        r##"# Summer leave and on-call cover

Planning holidays so customers always have someone to talk to.

## Requests so far

- Anna wants weeks 28 to 31 off.
- Erik prefers June plus the first week of July.
- Sara is flexible but wants at least three weeks in a row.

## Rules

At least two people work every week, and one of them must be able to handle incidents. The on-call rota keeps rotating, but shifts get longer because fewer people share them.

## Telling customers

Everyone gets a note well in advance about which weeks response times will be slower. Urgent issues go to the on-call phone, which is forwarded to whoever holds the rota that week.

- [ ] Put everyone's requests in the shared calendar by the end of April.
- [ ] Resolve overlaps at the next team meeting.
- [ ] Send the customer notice in May.
"##,
    ),
    (
        "Kardemummabullar",
        r##"# Kardemummabullar

Ett recept som räcker till ungefär trettio bullar, perfekt till fredagsfikat.

## Deg

- 50 g jäst
- 5 dl mjölk
- 150 g smör
- 1 dl socker
- 2 tsk nystött kardemumma
- en nypa salt
- cirka 13 dl vetemjöl

## Fyllning

- 150 g rumsvarmt smör
- 1 dl socker
- 1 msk kardemumma

## Gör så här

1. Smält smöret, häll i mjölken och värm till fingervarmt.
2. Rör ut jästen i lite av vätskan och blanda sedan i resten.
3. Tillsätt socker, salt, kardemumma och det mesta av mjölet. Knåda i tio minuter.
4. Låt jäsa under bakduk i en timme.
5. Kavla ut, bred på fyllningen, vik och skär remsor som du knyter till knutar.
6. Låt jäsa igen, pensla med ägg och strö över pärlsocker.
7. Grädda mitt i ugnen på 225 grader i cirka tio minuter.
"##,
    ),
    (
        "Half marathon training plan",
        r##"# Half marathon training plan

Twelve weeks from now there is a race, and the goal is simply to finish under two hours without getting injured.

## Weekly structure

- Monday: rest or a gentle walk.
- Tuesday: intervals, for example six times 800 metres at a hard but controlled pace.
- Thursday: an easy run of 45 minutes where talking is still possible.
- Saturday: the long run, adding about one kilometre each week up to 18.
- Sunday: mobility and strength work for hips and calves.

## Rules for myself

1. Never increase total weekly distance by more than ten percent.
2. Every fourth week is lighter so the body can recover.
3. Practise drinking and eating during the long runs, not on race day.

## Gear

The current shoes have more than 700 kilometres on them, so new ones should be bought now and broken in before the longest runs.
"##,
    ),
];

#[cfg(debug_assertions)]
const SAMPLES_FOLDER: &str = "Samples";

/// Adds [`SAMPLE_NOTES`] to a root `Samples` folder, skipping titles that already exist, so the
/// Seed button also fills a dev database that already has notes. Returns how many were added.
#[cfg(debug_assertions)]
pub(crate) async fn seed_sample_notes(pool: &Pool<Sqlite>) -> Result<u64, sqlx::Error> {
    let mut tx = pool.begin().await?;
    let mut missing = Vec::new();
    for (title, body) in SAMPLE_NOTES {
        let exists: Option<i64> =
            sqlx::query_scalar("SELECT id FROM notes WHERE Title = ? COLLATE NOCASE")
                .bind(title)
                .fetch_optional(&mut *tx)
                .await?;
        if exists.is_none() {
            missing.push((title, body));
        }
    }
    if missing.is_empty() {
        return Ok(0);
    }

    let existing_folder: Option<i64> = sqlx::query_scalar(
        "SELECT id FROM folders WHERE ParentId IS NULL AND Name = ? COLLATE NOCASE",
    )
    .bind(SAMPLES_FOLDER)
    .fetch_optional(&mut *tx)
    .await?;
    let folder_id =
        match existing_folder {
            Some(id) => id,
            None => sqlx::query_scalar(
                "INSERT INTO folders (ParentId, Name, Position) VALUES (NULL, ?, 0) RETURNING id",
            )
            .bind(SAMPLES_FOLDER)
            .fetch_one(&mut *tx)
            .await?,
        };

    let now = Local::now();
    for (age, (title, body)) in (1_i64..).zip(&missing) {
        let ts = (now - Duration::hours(age * 13))
            .format("%Y-%m-%dT%H:%M:%S%.3f")
            .to_string();
        sqlx::query(
            "INSERT INTO notes (FolderId, Title, Body, Pinned, CreatedAt, UpdatedAt) \
             VALUES (?, ?, ?, 0, ?, ?)",
        )
        .bind(folder_id)
        .bind(title)
        .bind(body)
        .bind(&ts)
        .bind(&ts)
        .execute(&mut *tx)
        .await?;
    }
    crate::commands::notes::rebuild_index(&mut tx).await?;

    tx.commit().await?;
    Ok(missing.len() as u64)
}

#[cfg(debug_assertions)]
pub async fn seed_test_data_if_empty(pool: &Pool<Sqlite>) -> Result<(), sqlx::Error> {
    seed_test_data_if_empty_with_anchor(pool, Utc::now().date_naive()).await
}

/// Testable inner function — seeds demo data if DB is empty.
#[cfg(debug_assertions)]
pub async fn inner_seed_database(pool: &Pool<Sqlite>) -> Result<String, AppError> {
    seed_test_data_if_empty(pool).await?;
    let added = seed_sample_notes(pool).await?;
    Ok(format!("Seed complete ({added} sample notes added)"))
}

#[cfg(debug_assertions)]
#[tauri::command]
pub async fn seed_database(
    pool: State<'_, Pool<Sqlite>>,
    semantic: State<'_, crate::semantic::SemanticState>,
) -> Result<String, AppError> {
    let message = inner_seed_database(&pool).await?;
    semantic.enqueue_all(pool.inner()).await?;
    Ok(message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    #[test]
    fn test_seed_start_date_covers_three_calendar_months() {
        let start = seed_start_date(NaiveDate::from_ymd_opt(2026, 4, 6).unwrap());
        assert_eq!(start.to_string(), "2026-02-01");
    }

    #[tokio::test]
    async fn test_seed_data_populates_empty_database() {
        let pool = crate::test_db().await;

        seed_test_data_if_empty_with_anchor(&pool, NaiveDate::from_ymd_opt(2026, 4, 6).unwrap())
            .await
            .unwrap();

        let customer_count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM customers")
            .fetch_one(&pool)
            .await
            .unwrap();
        let project_count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM projects")
            .fetch_one(&pool)
            .await
            .unwrap();
        let issue_count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM issues")
            .fetch_one(&pool)
            .await
            .unwrap();
        let entry_count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM time_entries")
            .fetch_one(&pool)
            .await
            .unwrap();
        let trip_count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM travel")
            .fetch_one(&pool)
            .await
            .unwrap();
        let defaults: Vec<(String, i64)> = sqlx::query_as(
            "SELECT Name, UseAsDefault FROM customers WHERE UseAsDefault = 1 ORDER BY Name",
        )
        .fetch_all(&pool)
        .await
        .unwrap();
        let seeded_range: (String, String) =
            sqlx::query_as("SELECT MIN(Date), MAX(Date) FROM time_entries")
                .fetch_one(&pool)
                .await
                .unwrap();

        assert_eq!(customer_count.0, 2, "expected two seeded customers");
        assert_eq!(project_count.0, 3, "expected three seeded projects");
        assert_eq!(
            issue_count.0, 7,
            "expected seeded issues across all projects"
        );
        assert_eq!(entry_count.0, 54, "expected the anchored seed volume");
        assert_eq!(trip_count.0, 7, "expected seeded trips");
        assert_eq!(
            defaults.len(),
            1,
            "exactly one default customer is expected"
        );
        assert_eq!(defaults[0].0, "Northwind Studio");
        assert_eq!(seeded_range.0, "2026-02-02");
        assert_eq!(seeded_range.1, "2026-04-06");
        assert_eq!(count(&pool, "notes").await, 6, "expected seeded notes");
        assert_eq!(count(&pool, "folders").await, 2, "expected seeded folders");
        let nested: (String, String, String) = sqlx::query_as(
            "SELECT p.Name, c.Name, n.Title FROM notes n
             JOIN folders c ON c.id = n.FolderId
             JOIN folders p ON p.id = c.ParentId",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(
            nested,
            ("Projects".into(), "Archive".into(), "Old roadmap".into())
        );
        let pinned: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM notes WHERE Pinned = 1")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(pinned.0, 1, "expected one pinned note");
        assert_eq!(
            count(&pool, "reminders").await,
            6,
            "expected seeded reminders"
        );
        let linked: (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM reminders WHERE NoteId IS NOT NULL")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(linked.0, 1, "expected one reminder linked to a note");
    }

    async fn count(pool: &Pool<Sqlite>, table: &str) -> i64 {
        let row: (i64,) = sqlx::query_as(&format!("SELECT COUNT(*) FROM {table}"))
            .fetch_one(pool)
            .await
            .unwrap();
        row.0
    }

    #[tokio::test]
    async fn test_seed_notes_skips_when_notes_exist() {
        let pool = crate::test_db().await;
        sqlx::query("INSERT INTO notes (Title, CreatedAt, UpdatedAt) VALUES ('Mine', 'x', 'x')")
            .execute(&pool)
            .await
            .unwrap();

        seed_test_data_if_empty_with_anchor(&pool, NaiveDate::from_ymd_opt(2026, 4, 6).unwrap())
            .await
            .unwrap();

        assert_eq!(count(&pool, "notes").await, 1);
        assert_eq!(count(&pool, "folders").await, 0);
        assert_eq!(
            count(&pool, "customers").await,
            2,
            "customers seed independently"
        );
    }

    #[tokio::test]
    async fn test_reseeding_notes_reuses_seeded_folders() {
        let pool = crate::test_db().await;
        let anchor = NaiveDate::from_ymd_opt(2026, 4, 6).unwrap();
        seed_test_data_if_empty_with_anchor(&pool, anchor)
            .await
            .unwrap();
        sqlx::query("DELETE FROM notes")
            .execute(&pool)
            .await
            .unwrap();

        seed_test_data_if_empty_with_anchor(&pool, anchor)
            .await
            .unwrap();

        assert_eq!(count(&pool, "notes").await, 6);
        assert_eq!(
            count(&pool, "folders").await,
            2,
            "folders are not duplicated"
        );
        let nested: (String, String) = sqlx::query_as(
            "SELECT c.Name, n.Title FROM notes n
             JOIN folders c ON c.id = n.FolderId
             JOIN folders p ON p.id = c.ParentId AND p.Name = 'Projects'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(nested, ("Archive".into(), "Old roadmap".into()));
    }

    #[tokio::test]
    async fn test_reseeding_notes_with_other_folders_seeds_at_root() {
        let pool = crate::test_db().await;
        sqlx::query("INSERT INTO folders (ParentId, Name, Position) VALUES (NULL, 'Mine', 0)")
            .execute(&pool)
            .await
            .unwrap();

        seed_test_data_if_empty_with_anchor(&pool, NaiveDate::from_ymd_opt(2026, 4, 6).unwrap())
            .await
            .unwrap();

        assert_eq!(count(&pool, "notes").await, 6);
        assert_eq!(count(&pool, "folders").await, 1, "no folders added");
        let in_folders: (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM notes WHERE FolderId IS NOT NULL")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(in_folders.0, 0, "seeded notes go to the root");
    }

    #[tokio::test]
    async fn test_seed_data_skips_non_empty_database() {
        let pool = crate::test_db().await;

        sqlx::query("INSERT INTO customers (Name, UseAsDefault) VALUES ('Existing Co', 0)")
            .execute(&pool)
            .await
            .unwrap();

        seed_test_data_if_empty_with_anchor(&pool, NaiveDate::from_ymd_opt(2026, 4, 6).unwrap())
            .await
            .unwrap();

        let customers: Vec<(String,)> = sqlx::query_as("SELECT Name FROM customers ORDER BY Name")
            .fetch_all(&pool)
            .await
            .unwrap();

        assert_eq!(customers, vec![("Existing Co".to_string(),)]);
        assert_eq!(
            count(&pool, "notes").await,
            6,
            "notes seed even when customers exist"
        );
        assert_eq!(count(&pool, "folders").await, 2);
    }

    #[tokio::test]
    async fn test_inner_seed_database_returns_ok() {
        let pool = crate::test_db().await;
        let result = inner_seed_database(&pool).await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "Seed complete (16 sample notes added)");
    }

    #[tokio::test]
    async fn test_sample_notes_fill_a_samples_folder_and_are_indexed() {
        let pool = crate::test_db().await;
        sqlx::query("INSERT INTO notes (Title, CreatedAt, UpdatedAt) VALUES ('Mine', 'x', 'x')")
            .execute(&pool)
            .await
            .unwrap();

        assert_eq!(seed_sample_notes(&pool).await.unwrap(), 16);

        let in_samples: Vec<(String, String)> = sqlx::query_as(
            "SELECT n.Title, n.Body FROM notes n JOIN folders f ON f.id = n.FolderId \
             WHERE f.Name = 'Samples' AND f.ParentId IS NULL",
        )
        .fetch_all(&pool)
        .await
        .unwrap();
        assert_eq!(in_samples.len(), 16);
        for (title, body) in &in_samples {
            let words = body.split_whitespace().count();
            assert!((100..=200).contains(&words), "{title}: {words} words");
        }
        let swedish = in_samples
            .iter()
            .filter(|(_, body)| body.contains(['å', 'ä', 'ö']))
            .count();
        assert!(
            swedish >= 7,
            "expected Swedish and English notes, got {swedish} Swedish"
        );
        assert_eq!(
            count(&pool, "notes_fts").await,
            17,
            "sample notes are searchable"
        );
    }

    #[tokio::test]
    async fn test_sample_notes_skip_existing_titles_and_reseed_idempotently() {
        let pool = crate::test_db().await;
        sqlx::query(
            "INSERT INTO notes (Title, Body, CreatedAt, UpdatedAt) \
             VALUES ('kardemummabullar', 'my own recipe', 'x', 'x')",
        )
        .execute(&pool)
        .await
        .unwrap();

        assert_eq!(seed_sample_notes(&pool).await.unwrap(), 15);
        assert_eq!(seed_sample_notes(&pool).await.unwrap(), 0);

        assert_eq!(count(&pool, "notes").await, 16);
        assert_eq!(count(&pool, "folders").await, 1);
        let mine: (String,) =
            sqlx::query_as("SELECT Body FROM notes WHERE Title = 'kardemummabullar'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(mine.0, "my own recipe");
    }
}
