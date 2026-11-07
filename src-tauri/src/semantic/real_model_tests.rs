//! Opt-in tests against the real e5 model. They never download anything: run them after
//! enabling semantic search in a debug build (files in `.timetracker-dev/models/`) or
//! point `TIMETRACKER_MODEL_DIR` at a directory holding the model files:
//!
//! ```sh
//! cargo test -p time-tracker real_model -- --ignored --nocapture --test-threads=1
//! ```
//!
//! `real_model_spike` also compares against the fp32 export when `model.onnx` is present
//! in the same directory, and prints the numbers recorded in the brief.

use super::chunker::{chunk_note, CHUNK_TOKENS};
use super::embedder::{dot, passage_text, query_text, Embedder, OrtEmbedder};
use super::model_files;
use std::path::PathBuf;
use std::time::Instant;

fn model_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("TIMETRACKER_MODEL_DIR") {
        return PathBuf::from(dir);
    }
    let paths = crate::runtime_paths::runtime_paths(std::path::Path::new("/nonexistent-home"));
    model_files::model_dir(&paths.models_dir)
}

fn load(file: &str) -> OrtEmbedder {
    let dir = model_dir();
    OrtEmbedder::load(
        &dir.join(file),
        &dir.join(model_files::TOKENIZER_FILE),
        model_files::MODEL_ID,
    )
    .unwrap_or_else(|e| panic!("load {file} from {dir:?}: {e}"))
}

#[test]
#[ignore = "needs the downloaded model; see module docs"]
fn real_model_matches_across_languages() {
    let e = load(model_files::MODEL_FILE);
    let texts = [
        passage_text("Mötet om budgeten för nästa år flyttas till torsdag."),
        passage_text("The meeting about next year's budget is moved to Thursday."),
        passage_text("Tomatplantorna behöver vattnas varje kväll i juli."),
    ];
    let v = e.embed(&texts).unwrap();
    let cross = dot(&v[0], &v[1]);
    let unrelated = dot(&v[0], &v[2]);
    println!("cross-language {cross:.3}, unrelated {unrelated:.3}");
    assert!(cross > unrelated, "{cross} <= {unrelated}");
    let q = e.embed(&[query_text("budget meeting")]).unwrap();
    assert!(dot(&q[0], &v[0]) > dot(&q[0], &v[2]));
}

/// The int8 export quantises activations per inference run (DynamicQuantizeLinear), so a
/// text embedded next to others would get a slightly different vector (cosine ~0.97).
/// Every input must run alone: a chunk stored by the indexer, or embedded in one call
/// with other texts, must match the same text embedded on its own.
#[tokio::test]
#[ignore = "needs the downloaded model; see module docs"]
async fn real_model_embeds_each_input_alone() {
    use super::index::decode_vector;
    use super::SemanticState;
    use std::sync::Arc;

    let e: Arc<dyn Embedder> = Arc::new(load(model_files::MODEL_FILE));
    let alone = |text: &str| e.embed(&[text.to_string()]).unwrap().remove(0);
    let close = |a: &[f32], b: &[f32], what: &str| {
        let cos = dot(a, b);
        println!("{what}: cosine {cos:.6}");
        assert!(cos > 0.9999, "{what}: cosine {cos}");
    };

    // One call with several (different-length) texts.
    let texts = [
        passage_text("Mötet om budgeten för nästa år flyttas till torsdag."),
        passage_text(
            "Tomatplantorna behöver vattnas varje kväll i juli, särskilt när det är varmt.",
        ),
        query_text("budget meeting"),
    ];
    let together = e.embed(&texts).unwrap();
    for (text, v) in texts.iter().zip(&together) {
        close(v, &alone(text), "embedded in one call with others");
    }

    // The indexing path: a note of several chunks, each stored with neighbours around it.
    let pool = crate::test_db().await;
    let body: String = TOPICS
        .iter()
        .take(4)
        .map(|(topic, sv, en, _, _)| format!("## {topic}\n\n{} {}\n\n", sv.join(" "), en.join(" ")))
        .collect();
    let note = crate::commands::notes::inner_create_note_titled("Veckoanteckningar", None, &pool)
        .await
        .unwrap()
        .id;
    crate::commands::notes::inner_update_note_body(note, &body, &pool)
        .await
        .unwrap();
    let state = SemanticState::new(PathBuf::from("/nonexistent"));
    state.set_enabled(true);
    state.set_embedder(Some(e.clone()));
    state.enqueue([note]);
    state.drain(&pool).await;

    let chunks = chunk_note("Veckoanteckningar", &body, CHUNK_TOKENS, &|t| {
        e.count_tokens(t)
    });
    let stored: Vec<(i64, Vec<u8>)> = sqlx::query_as(
        "SELECT ChunkIndex AS chunk_index, Vector AS vector FROM note_embeddings \
         WHERE NoteId = ? ORDER BY ChunkIndex",
    )
    .bind(note)
    .fetch_all(&pool)
    .await
    .unwrap();
    assert!(chunks.len() >= 3, "{} chunks", chunks.len());
    assert_eq!(stored.len(), chunks.len());
    for (chunk, (index, bytes)) in chunks.iter().zip(&stored) {
        let what = format!("indexed chunk {index}");
        close(
            &decode_vector(bytes),
            &alone(&passage_text(&chunk.input)),
            &what,
        );
    }
}

/// (topic, Swedish sentences, English sentences, Swedish query, English query)
type Topic = (
    &'static str,
    [&'static str; 4],
    [&'static str; 4],
    &'static str,
    &'static str,
);

const TOPICS: [Topic; 20] = [
    (
        "budget",
        [
            "Budgeten för nästa år måste vara klar före november.",
            "Vi lägger tjugo procent mer på marknadsföring än i fjol.",
            "Ekonomichefen vill se en prognos per kvartal.",
            "Resekostnaderna sticker ut och behöver skäras ner.",
        ],
        [
            "Next year's budget has to be finished before November.",
            "We are spending twenty percent more on marketing than last year.",
            "The CFO wants to see a forecast per quarter.",
            "Travel costs stand out and need to be cut.",
        ],
        "hur mycket pengar får vi spendera",
        "how much money can we spend next year",
    ),
    (
        "outage",
        [
            "Servern gick ner klockan tre i natt efter en diskkrasch.",
            "Databasen var otillgänglig i fyrtio minuter.",
            "Vi återställde från gårdagens säkerhetskopia.",
            "Larmet nådde inte jourhavande eftersom telefonen var avstängd.",
        ],
        [
            "The server went down at three last night after a disk crash.",
            "The database was unavailable for forty minutes.",
            "We restored from yesterday's backup.",
            "The alert never reached the on-call engineer because the phone was off.",
        ],
        "varför slutade systemet fungera i natt",
        "why did the system stop working overnight",
    ),
    (
        "recipe",
        [
            "Fräs löken i smör tills den blir mjuk.",
            "Tillsätt krossade tomater och låt puttra i tjugo minuter.",
            "Smaka av med salt, peppar och lite socker.",
            "Servera med nykokt pasta och riven parmesan.",
        ],
        [
            "Fry the onion in butter until it is soft.",
            "Add crushed tomatoes and let it simmer for twenty minutes.",
            "Season with salt, pepper and a little sugar.",
            "Serve with freshly cooked pasta and grated parmesan.",
        ],
        "vad ska jag laga till middag",
        "what should I cook for dinner",
    ),
    (
        "hiring",
        [
            "Vi behöver anställa två nya utvecklare till hösten.",
            "Annonsen ligger ute på LinkedIn i tre veckor.",
            "Intervjuerna sker i två steg med ett kodtest.",
            "Kandidaten från Göteborg hade bäst referenser.",
        ],
        [
            "We need to hire two new developers for the autumn.",
            "The job ad is posted on LinkedIn for three weeks.",
            "Interviews happen in two rounds with a coding test.",
            "The candidate from Gothenburg had the best references.",
        ],
        "rekrytering av personal",
        "recruiting new staff for the team",
    ),
    (
        "running",
        [
            "Långpasset på söndag blir tjugo kilometer i lugnt tempo.",
            "Intervallerna på tisdag är sex gånger tusen meter.",
            "Vaden kändes stel efter backträningen.",
            "Målet är att springa maraton under fyra timmar.",
        ],
        [
            "The long run on Sunday will be twenty kilometres at an easy pace.",
            "Tuesday's intervals are six times one thousand metres.",
            "My calf felt stiff after the hill training.",
            "The goal is to run a marathon in under four hours.",
        ],
        "träningsprogram inför loppet",
        "training plan before the race",
    ),
    (
        "car",
        [
            "Bilen ska in på service på måndag morgon.",
            "Bromsbeläggen fram är slitna och måste bytas.",
            "Verkstaden ringde om att kamremmen också borde bytas.",
            "Notan landade på nästan åtta tusen kronor.",
        ],
        [
            "The car goes in for a service on Monday morning.",
            "The front brake pads are worn and must be replaced.",
            "The garage called saying the timing belt should be changed too.",
            "The bill came to almost eight thousand kronor.",
        ],
        "reparation av fordonet",
        "fixing the vehicle at the mechanic",
    ),
    (
        "tax",
        [
            "Deklarationen ska lämnas in senast den andra maj.",
            "Glöm inte att dra av kostnaderna för hemmakontoret.",
            "Skatteverket har redan fyllt i lönen.",
            "Ränteavdraget för bolånet blev lägre i år.",
        ],
        [
            "The tax return must be filed by the second of May.",
            "Don't forget to deduct the home office costs.",
            "The tax agency has already filled in the salary.",
            "The interest deduction on the mortgage was lower this year.",
        ],
        "skatter och avdrag",
        "paying the government on income",
    ),
    (
        "school",
        [
            "Utvecklingssamtalet med läraren är på onsdag.",
            "Barnen behöver nya gympakläder till terminen.",
            "Skolan har studiedag fredagen efter lovet.",
            "Läxan i matte handlar om bråktal.",
        ],
        [
            "The parent-teacher meeting is on Wednesday.",
            "The kids need new gym clothes for the term.",
            "The school has a staff training day the Friday after the break.",
            "The maths homework is about fractions.",
        ],
        "barnens skolgång",
        "children's education and teachers",
    ),
    (
        "invoice",
        [
            "Fakturan till kunden skickades den sista i månaden.",
            "Betalningsvillkoren är trettio dagar netto.",
            "Kunden har inte betalat och en påminnelse har gått ut.",
            "Timmarna för support debiteras separat.",
        ],
        [
            "The invoice to the customer was sent on the last day of the month.",
            "Payment terms are thirty days net.",
            "The customer has not paid and a reminder has been sent.",
            "Support hours are billed separately.",
        ],
        "kunden betalar inte",
        "client owes us money",
    ),
    (
        "vacation",
        [
            "Vi hyr en stuga i Dalarna i två veckor i juli.",
            "Tåget går från Stockholm tidigt på morgonen.",
            "Glöm inte att boka båten ut till ön.",
            "Grannen vattnar blommorna medan vi är borta.",
        ],
        [
            "We are renting a cabin in Dalarna for two weeks in July.",
            "The train leaves Stockholm early in the morning.",
            "Don't forget to book the boat out to the island.",
            "The neighbour waters the plants while we are away.",
        ],
        "planer för sommarledigheten",
        "holiday plans for the summer",
    ),
    (
        "migration",
        [
            "Databasmigreringen lägger till en ny kolumn i tabellen.",
            "Vi måste köra migreringen i en transaktion.",
            "Gamla rader fylls i med ett standardvärde.",
            "Testet bygger det gamla schemat och migrerar det.",
        ],
        [
            "The database migration adds a new column to the table.",
            "We have to run the migration in a transaction.",
            "Old rows are filled with a default value.",
            "The test builds the old schema and migrates it.",
        ],
        "ändra schemat i sqlite",
        "changing the schema of existing tables",
    ),
    (
        "complaint",
        [
            "Kunden klagade på att leveransen var två veckor sen.",
            "Varan var skadad när paketet öppnades.",
            "Vi erbjöd en rabatt på nästa beställning.",
            "Kundtjänst svarade inte på mejlet på fem dagar.",
        ],
        [
            "The customer complained that the delivery was two weeks late.",
            "The item was damaged when the parcel was opened.",
            "We offered a discount on the next order.",
            "Customer service did not answer the email for five days.",
        ],
        "missnöjd kund",
        "unhappy buyer feedback",
    ),
    (
        "garden",
        [
            "Tomatplantorna behöver vattnas varje kväll i juli.",
            "Rabatten vid staketet ska få nya perenner.",
            "Gräsmattan har fått mossa och måste luftas.",
            "Äppelträdet beskärs bäst i februari.",
        ],
        [
            "The tomato plants need watering every evening in July.",
            "The bed by the fence will get new perennials.",
            "The lawn has moss and needs aerating.",
            "The apple tree is best pruned in February.",
        ],
        "skötsel av trädgården",
        "looking after plants outdoors",
    ),
    (
        "onboarding",
        [
            "Den nya kollegan börjar på måndag och behöver en dator.",
            "Första veckan går till att sätta upp utvecklingsmiljön.",
            "En mentor visar hur vi granskar kod.",
            "Behörigheter till produktionsmiljön ges efter en månad.",
        ],
        [
            "The new colleague starts on Monday and needs a laptop.",
            "The first week is spent setting up the development environment.",
            "A mentor shows how we review code.",
            "Access to production is granted after a month.",
        ],
        "introduktion av nyanställda",
        "welcoming a new team member",
    ),
    (
        "marketing",
        [
            "Kampanjen i sociala medier startar i mars.",
            "Vi testar två olika rubriker på annonserna.",
            "Nyhetsbrevet har fått fler prenumeranter.",
            "Mässan i Malmö kostar mycket men ger många kontakter.",
        ],
        [
            "The social media campaign starts in March.",
            "We are testing two different headlines on the ads.",
            "The newsletter has gained more subscribers.",
            "The trade fair in Malmö is expensive but brings many leads.",
        ],
        "nå fler kunder med reklam",
        "advertising to attract customers",
    ),
    (
        "book",
        [
            "Romanen utspelar sig i ett fiskeläge på västkusten.",
            "Huvudpersonen återvänder hem efter tjugo år.",
            "Språket är vackert men handlingen går långsamt.",
            "Bokcirkeln träffas nästa gång hos Anna.",
        ],
        [
            "The novel is set in a fishing village on the west coast.",
            "The main character returns home after twenty years.",
            "The language is beautiful but the plot moves slowly.",
            "The book club meets at Anna's next time.",
        ],
        "vad tyckte jag om berättelsen",
        "thoughts on the story I read",
    ),
    (
        "renovation",
        [
            "Badrummet ska totalrenoveras under hösten.",
            "Kakelsättaren kan börja först i oktober.",
            "Vi valde gråa klinkers och vita väggar.",
            "Rörmokaren flyttar golvbrunnen närmare duschen.",
        ],
        [
            "The bathroom will be completely renovated in the autumn.",
            "The tiler can only start in October.",
            "We chose grey floor tiles and white walls.",
            "The plumber is moving the floor drain closer to the shower.",
        ],
        "bygga om hemma",
        "remodelling the apartment",
    ),
    (
        "doctor",
        [
            "Läkaren tog blodprov och tittade på halsen.",
            "Jag ska ta antibiotika i tio dagar.",
            "Återbesöket är bokat om tre veckor.",
            "Febern gick ner efter två dagar.",
        ],
        [
            "The doctor took blood samples and looked at my throat.",
            "I have to take antibiotics for ten days.",
            "The follow-up visit is booked in three weeks.",
            "The fever went down after two days.",
        ],
        "när jag var sjuk",
        "visit to the clinic when ill",
    ),
    (
        "release",
        [
            "Nya versionen släpps på torsdag efter granskningen.",
            "Byggservern skapar en signerad installationsfil.",
            "Ändringsloggen beskriver tre nya funktioner.",
            "Vi rullar tillbaka om kraschrapporterna ökar.",
        ],
        [
            "The new version ships on Thursday after the review.",
            "The build server produces a signed installer.",
            "The changelog describes three new features.",
            "We roll back if crash reports go up.",
        ],
        "publicera appen till användarna",
        "shipping the app to users",
    ),
    (
        "moving",
        [
            "Flyttfirman kommer klockan åtta på lördag.",
            "Alla böcker är packade i tjugo kartonger.",
            "Vi måste säga upp elavtalet i den gamla lägenheten.",
            "Adressändringen görs hos Skatteverket.",
        ],
        [
            "The removal company arrives at eight on Saturday.",
            "All the books are packed in twenty boxes.",
            "We have to cancel the electricity contract at the old flat.",
            "The change of address is done with the tax agency.",
        ],
        "byta bostad",
        "relocating to a new home",
    ),
];

/// 200 passages (10 per topic: 5 Swedish, 5 English) and their topic index.
fn corpus() -> Vec<(usize, String)> {
    let mut out = Vec::new();
    let picks: [&[usize]; 5] = [
        &[0, 1, 2],
        &[1, 2, 3],
        &[0, 2, 3],
        &[0, 1, 3],
        &[0, 1, 2, 3],
    ];
    for (t, (_, sv, en, _, _)) in TOPICS.iter().enumerate() {
        for lang in [sv, en] {
            for pick in picks {
                let text: Vec<&str> = pick.iter().map(|&i| lang[i]).collect();
                out.push((t, text.join(" ")));
            }
        }
    }
    out
}

fn queries() -> Vec<(usize, String)> {
    TOPICS
        .iter()
        .enumerate()
        .flat_map(|(t, (_, _, _, sv, en))| [(t, sv.to_string()), (t, en.to_string())])
        .collect()
}

fn top_k(q: &[f32], docs: &[Vec<f32>], k: usize) -> Vec<usize> {
    let mut scored: Vec<(usize, f32)> = docs.iter().map(|d| dot(q, d)).enumerate().collect();
    scored.sort_by(|a, b| b.1.total_cmp(&a.1));
    scored.into_iter().take(k).map(|(i, _)| i).collect()
}

/// Each text runs alone inside `embed` (see `OrtEmbedder`), as in the app.
fn embed_all(e: &OrtEmbedder, texts: &[String]) -> Vec<Vec<f32>> {
    e.embed(texts).unwrap()
}

fn percentile(sorted: &[f32], p: f32) -> f32 {
    let ix = ((sorted.len() - 1) as f32 * p).round() as usize;
    sorted[ix]
}

/// Neutral note titles and section headings (no topic words), Swedish and English.
const TITLES: [&str; 10] = [
    "Anteckningar",
    "Notes",
    "Möte",
    "Meeting notes",
    "Inkorg",
    "Veckoplanering",
    "Weekly review",
    "Idéer",
    "Att göra",
    "Journal",
];
const HEADINGS: [&str; 10] = [
    "Bakgrund",
    "Status",
    "Nästa steg",
    "Detaljer",
    "Frågor",
    "Background",
    "Summary",
    "Next steps",
    "Details",
    "Open questions",
];
/// Notes with at least this many chunks are the "long" bucket (most chances to pass by noise).
const LONG_NOTE_CHUNKS: usize = 4;

/// A synthetic markdown note and the topics its passages come from.
struct SynthNote {
    topics: Vec<usize>,
    title: String,
    body: String,
}

/// Synthetic markdown notes over the corpus passages, chunked later exactly as the app
/// does (title + heading path prefix, ~200 tokens). 1–10 passages per note as
/// paragraphs; 70% of notes cover one topic, 30% two. Half the titles are neutral
/// ("Möte 14/3"), half name the first topic ("Budget – Notes"); 60% of notes put
/// `##`/`###` headings (neutral) before some paragraphs. Deterministic (LCG).
fn notes(corpus: &[(usize, String)], count: usize) -> Vec<SynthNote> {
    let mut seed: u64 = 0x5eed_1234_abcd_0001;
    let mut next = |n: usize| {
        seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        ((seed >> 33) as usize) % n
    };
    let topics = TOPICS.len();
    (0..count)
        .map(|_| {
            let size = 1 + next(10);
            let mut note_topics = vec![next(topics)];
            if next(10) < 3 {
                let other = (note_topics[0] + 1 + next(topics - 1)) % topics;
                note_topics.push(other);
            }
            let mut pool: Vec<usize> = (0..corpus.len())
                .filter(|&i| note_topics.contains(&corpus[i].0))
                .collect();
            let neutral = TITLES[next(TITLES.len())];
            let title = if next(2) == 0 {
                format!("{neutral} {}/{}", 1 + next(28), 1 + next(12))
            } else {
                let key = TOPICS[note_topics[0]].0;
                let mut cap = key[..1].to_uppercase();
                cap.push_str(&key[1..]);
                format!("{cap} – {neutral}")
            };
            let with_headings = next(10) < 6;
            let mut body = String::new();
            for i in 0..size {
                if pool.is_empty() {
                    break;
                }
                if with_headings && next(2) == 0 {
                    let level = if i > 0 && next(3) == 0 { "###" } else { "##" };
                    body.push_str(&format!("{level} {}\n\n", HEADINGS[next(HEADINGS.len())]));
                }
                let passage = pool.swap_remove(next(pool.len()));
                body.push_str(&corpus[passage].1);
                body.push_str("\n\n");
            }
            SynthNote {
                topics: note_topics,
                title,
                body,
            }
        })
        .collect()
}

/// Chunks each note with the app's chunker and real token counts, and embeds every chunk
/// as the indexer does (`passage: ` + title/heading prefix + body).
fn embed_notes(e: &OrtEmbedder, notes: &[SynthNote]) -> Vec<Vec<Vec<f32>>> {
    let inputs: Vec<Vec<String>> = notes
        .iter()
        .map(|n| {
            chunk_note(&n.title, &n.body, CHUNK_TOKENS, &|t| e.count_tokens(t))
                .iter()
                .map(|c| passage_text(&c.input))
                .collect()
        })
        .collect();
    println!("example chunk input: {:?}", inputs[0][0]);
    let flat: Vec<String> = inputs.iter().flatten().cloned().collect();
    let s = Instant::now();
    let mut vectors = embed_all(e, &flat).into_iter();
    println!("note chunks: {} embedded in {:?}", flat.len(), s.elapsed());
    inputs
        .iter()
        .map(|chunks| vectors.by_ref().take(chunks.len()).collect())
        .collect()
}

/// The floor at note level, where a note scores its best chunk: the share of unrelated
/// notes that pass (noise grows with chunk count) against the share of queries that keep
/// a relevant note, over a range of floors.
fn note_level_floor(
    notes: &[SynthNote],
    note_vectors: &[Vec<Vec<f32>>],
    queries: &[(usize, String)],
    qv: &[Vec<f32>],
    current: f32,
) {
    let sizes: Vec<usize> = note_vectors.iter().map(Vec::len).collect();
    let mut histogram = vec![0usize; sizes.iter().max().copied().unwrap_or(0) + 1];
    for n in &sizes {
        histogram[*n] += 1;
    }
    println!(
        "note level: {} notes, {} chunks (mean {:.1}; notes per chunk count {:?})",
        notes.len(),
        sizes.iter().sum::<usize>(),
        sizes.iter().sum::<usize>() as f32 / sizes.len() as f32,
        &histogram[1..]
    );
    // Per query: (note score, relevant, chunk count).
    let scored: Vec<Vec<(f32, bool, usize)>> = queries
        .iter()
        .zip(qv)
        .map(|((qt, _), q)| {
            notes
                .iter()
                .zip(note_vectors)
                .map(|(note, chunks)| {
                    let best = chunks.iter().map(|c| dot(q, c)).fold(f32::MIN, f32::max);
                    (best, note.topics.contains(qt), chunks.len())
                })
                .collect()
        })
        .collect();
    let mut unrelated: Vec<f32> = scored
        .iter()
        .flatten()
        .filter(|(_, rel, _)| !rel)
        .map(|(s, _, _)| *s)
        .collect();
    unrelated.sort_by(f32::total_cmp);
    let mut best_relevant: Vec<f32> = scored
        .iter()
        .map(|notes| {
            notes
                .iter()
                .filter(|(_, rel, _)| *rel)
                .map(|(s, _, _)| *s)
                .fold(f32::MIN, f32::max)
        })
        .collect();
    best_relevant.sort_by(f32::total_cmp);
    println!(
        "unrelated notes: p50 {:.3} p95 {:.3} p98 {:.3} p99 {:.3} max {:.3}; \
         best relevant note per query: p05 {:.3} p50 {:.3}",
        percentile(&unrelated, 0.5),
        percentile(&unrelated, 0.95),
        percentile(&unrelated, 0.98),
        percentile(&unrelated, 0.99),
        unrelated[unrelated.len() - 1],
        percentile(&best_relevant, 0.05),
        percentile(&best_relevant, 0.5),
    );
    // Noise among the longest notes, the worst case.
    let long: Vec<f32> = scored
        .iter()
        .flatten()
        .filter(|(_, rel, n)| !rel && *n >= LONG_NOTE_CHUNKS)
        .map(|(s, _, _)| *s)
        .collect();
    let mut floors: Vec<f32> = (0..=28)
        .map(|i| 0.78 + i as f32 * 0.0025)
        .filter(|f| (f - current).abs() > 1e-4)
        .collect();
    floors.push(current);
    floors.sort_by(f32::total_cmp);
    let mut lowest_ok = None;
    for floor in floors {
        let noise =
            unrelated.iter().filter(|s| **s >= floor).count() as f32 / unrelated.len() as f32;
        let long_noise =
            long.iter().filter(|s| **s >= floor).count() as f32 / long.len().max(1) as f32;
        let with_hit = scored
            .iter()
            .filter(|notes| notes.iter().any(|(s, rel, _)| *rel && *s >= floor))
            .count() as f32
            / queries.len() as f32;
        let (kept, kept_relevant) = scored
            .iter()
            .flatten()
            .filter(|(s, _, _)| *s >= floor)
            .fold((0usize, 0usize), |(k, r), (_, rel, _)| {
                (k + 1, r + usize::from(*rel))
            });
        if noise <= 0.02 && lowest_ok.is_none() {
            lowest_ok = Some(floor);
        }
        println!(
            "note floor {floor:.4}: unrelated notes kept {:.1}% (≥{LONG_NOTE_CHUNKS} chunks {:.1}%), \
             queries with a relevant note {:.1}%, precision of kept {:.2}{}",
            noise * 100.0,
            long_noise * 100.0,
            with_hit * 100.0,
            kept_relevant as f32 / kept.max(1) as f32,
            if floor == current {
                "  <- SIMILARITY_FLOOR"
            } else {
                ""
            }
        );
    }
    match lowest_ok {
        Some(f) => println!("lowest floor with ≤ 2% unrelated notes: {f:.4}"),
        None => println!("no floor in the sweep keeps unrelated notes ≤ 2%"),
    }
}

#[test]
#[ignore = "needs the downloaded model; see module docs"]
fn real_model_spike() {
    let corpus = corpus();
    let queries = queries();
    let passages: Vec<String> = corpus.iter().map(|(_, t)| passage_text(t)).collect();
    let qtexts: Vec<String> = queries.iter().map(|(_, t)| query_text(t)).collect();

    let started = Instant::now();
    let e = load(model_files::MODEL_FILE);
    println!("load {:?}", started.elapsed());

    // Speed: one ~200-token chunk at a time (what the indexer does at worst).
    let long: Vec<String> = TOPICS
        .iter()
        .take(12)
        .map(|(_, sv, en, _, _)| {
            let mut text = String::from("passage: ");
            let sentences: Vec<&str> = sv.iter().chain(en.iter()).copied().collect();
            let mut i = 0;
            while e.count_tokens(&text) < 200 {
                text.push_str(sentences[i % sentences.len()]);
                text.push(' ');
                i += 1;
            }
            text
        })
        .collect();
    let mut ms: Vec<f64> = Vec::new();
    for t in &long {
        let tokens = e.count_tokens(t);
        let s = Instant::now();
        e.embed(std::slice::from_ref(t)).unwrap();
        ms.push(s.elapsed().as_secs_f64() * 1000.0);
        println!("chunk {tokens} tokens: {:.0} ms", ms.last().unwrap());
    }
    let mut sorted = ms.clone();
    sorted.sort_by(f64::total_cmp);
    println!(
        "median ms/chunk (~200 tokens): {:.0}",
        sorted[sorted.len() / 2]
    );
    let mut qms: Vec<f64> = Vec::new();
    for q in queries.iter().take(10) {
        let s = Instant::now();
        e.embed(&[query_text(&q.1)]).unwrap();
        qms.push(s.elapsed().as_secs_f64() * 1000.0);
    }
    qms.sort_by(f64::total_cmp);
    println!("median ms/query: {:.0}", qms[qms.len() / 2]);
    let s = Instant::now();
    e.embed(&long[..8]).unwrap();
    println!(
        "8 chunks in one embed call (one run each): {:.0} ms",
        s.elapsed().as_secs_f64() * 1000.0
    );

    let s = Instant::now();
    let docs = embed_all(&e, &passages);
    println!("corpus of {} passages: {:?}", docs.len(), s.elapsed());
    let qv = embed_all(&e, &qtexts);
    let synth = notes(&corpus, 400);
    let note_vectors = embed_notes(&e, &synth);
    drop(e);

    // Similarity distributions for the floor: same topic vs other topics.
    let mut relevant = Vec::new();
    let mut unrelated = Vec::new();
    let mut best_relevant = Vec::new();
    let mut hits_at_10 = 0;
    for ((qt, _), q) in queries.iter().zip(&qv) {
        let mut best = f32::MIN;
        for ((dt, _), d) in corpus.iter().zip(&docs) {
            let s = dot(q, d);
            if dt == qt {
                relevant.push(s);
                best = best.max(s);
            } else {
                unrelated.push(s);
            }
        }
        best_relevant.push(best);
        hits_at_10 += top_k(q, &docs, 10)
            .iter()
            .filter(|&&i| corpus[i].0 == *qt)
            .count();
    }
    for v in [&mut relevant, &mut unrelated, &mut best_relevant] {
        v.sort_by(f32::total_cmp);
    }
    println!(
        "precision@10 {:.2}",
        hits_at_10 as f32 / (10 * queries.len()) as f32
    );
    for (name, v) in [
        ("relevant", &relevant),
        ("unrelated", &unrelated),
        ("best relevant per query", &best_relevant),
    ] {
        println!(
            "{name}: min {:.3} p05 {:.3} p50 {:.3} p95 {:.3} p99 {:.3} max {:.3}",
            v[0],
            percentile(v, 0.05),
            percentile(v, 0.5),
            percentile(v, 0.95),
            percentile(v, 0.99),
            v[v.len() - 1]
        );
    }

    // What the floor keeps per raw passage pair (no title prefix; the note-level sweep
    // below, over app-chunked notes, is the calibration).
    let floor = super::fusion::SIMILARITY_FLOOR;
    let share = |v: &[f32]| v.iter().filter(|s| **s >= floor).count() as f32 / v.len() as f32;
    println!(
        "floor {floor}: keeps {:.1}% of unrelated pairs, {:.1}% of relevant pairs, \
         a relevant hit for {:.1}% of queries",
        share(&unrelated) * 100.0,
        share(&relevant) * 100.0,
        share(&best_relevant) * 100.0
    );

    note_level_floor(&synth, &note_vectors, &queries, &qv, floor);

    // int8 vs fp32 top-10 agreement, when the fp32 export is present.
    let fp32 = model_dir().join("model.onnx");
    if fp32.exists() {
        let e = load("model.onnx");
        let docs32 = embed_all(&e, &passages);
        let qv32 = embed_all(&e, &qtexts);
        let mut overlap = 0.0;
        for (q8, q32) in qv.iter().zip(&qv32) {
            let a = top_k(q8, &docs, 10);
            let b = top_k(q32, &docs32, 10);
            overlap += a.iter().filter(|i| b.contains(i)).count() as f64 / 10.0;
        }
        println!(
            "int8 vs fp32 top-10 overlap: {:.3}",
            overlap / queries.len() as f64
        );
    }
}
