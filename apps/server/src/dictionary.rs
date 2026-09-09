use super::*;

/// Word lookup across the three bundled dictionaries. Pure text and SQLite/MDX munging:
/// no HTTP types, no database pool, no auth.

#[derive(Deserialize)]
pub(crate) struct DictionaryQuery {
    word: String,
    dictionary: String,
}
#[derive(Serialize)]
pub(crate) struct DictionaryEntryResponse {
    headword: String,
    lemma: String,
    pronunciation: String,
    definitions: Vec<String>,
    translations: Vec<String>,
    forms: Vec<String>,
    labels: Vec<String>,
    examples: Vec<String>,
    detail: Value,
    definition_html: String,
    matched_terms: Vec<String>,
}
#[derive(Serialize)]
pub(crate) struct DictionaryResponse {
    word: String,
    dictionary: String,
    entries: Vec<DictionaryEntryResponse>,
}

pub(crate) async fn dictionary_lookup(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<DictionaryQuery>,
) -> Result<Json<DictionaryResponse>, ApiError> {
    user_from_headers(&state, &headers)?;
    let word = query.word.trim().to_string();
    if word.is_empty() || word.chars().count() > 120 {
        return Err(ApiError::bad(
            "Select a word or short phrase of up to 120 characters.",
        ));
    }
    let dictionary = query.dictionary;
    if !matches!(
        dictionary.as_str(),
        "russian_en" | "german_en" | "english_zh"
    ) {
        return Err(ApiError::bad("Unsupported dictionary."));
    }
    let directory = (*state.dictionary_dir).clone();
    let russian_dictionary = Arc::clone(&state.russian_dictionary);
    let response = tokio::task::spawn_blocking(move || {
        lookup_dictionary(&directory, &russian_dictionary, &dictionary, &word)
    })
    .await
    .map_err(|_| ApiError::internal("Dictionary worker did not complete."))??;
    Ok(Json(response))
}

fn lookup_dictionary(
    directory: &std::path::Path,
    russian_dictionary: &Mutex<Mdx>,
    dictionary: &str,
    word: &str,
) -> Result<DictionaryResponse, ApiError> {
    if !directory.is_dir() {
        return Err(ApiError::internal(
            "Local dictionaries are not installed on this server.",
        ));
    }
    let entries = match dictionary {
        "russian_en" => lookup_russian_dictionary(russian_dictionary, word)?,
        "german_en" => lookup_german_dictionary(directory, word)?,
        "english_zh" => lookup_english_chinese_dictionary(directory, word)?,
        _ => unreachable!(),
    };
    Ok(DictionaryResponse {
        word: word.to_string(),
        dictionary: dictionary.to_string(),
        entries,
    })
}

fn normalize_dictionary_key(value: &str) -> String {
    value
        .trim()
        .replace('\u{0301}', "")
        .to_lowercase()
        .chars()
        .filter(|character| character.is_alphanumeric() || *character == 'ё')
        .collect()
}

fn split_lines(value: &str) -> Vec<String> {
    value
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect()
}

fn lookup_russian_dictionary(
    dictionary: &Mutex<Mdx>,
    word: &str,
) -> Result<Vec<DictionaryEntryResponse>, ApiError> {
    let mut mdx = dictionary
        .lock()
        .map_err(|_| ApiError::internal("Russian dictionary is unavailable."))?;
    let mut terms = vec![word.trim().to_lowercase()];
    let alternate = terms[0].replace('ё', "е");
    if alternate != terms[0] {
        terms.push(alternate);
    }
    for term in terms {
        if let Some(entry) = russian_entry(&mut mdx, &term, None) {
            return Ok(vec![entry]);
        }
    }
    Ok(Vec::new())
}

fn russian_entry(
    mdx: &mut Mdx,
    term: &str,
    linked_from: Option<&str>,
) -> Option<DictionaryEntryResponse> {
    let normalized = normalize_dictionary_key(term);
    let entries = russian_exact_entries(mdx, &normalized);
    for item in &entries {
        let lookup = mdx.fetch(item)?;
        if let Some(target) = russian_link_target(&lookup.definition) {
            if normalize_dictionary_key(&target) != normalized {
                return russian_entry(mdx, &target, Some(&lookup.key_text));
            }
        }
        return russian_response(lookup.key_text, &lookup.definition, linked_from);
    }
    let lookup = mdx.lookup(term)?;
    if let Some(target) = russian_link_target(&lookup.definition) {
        if normalize_dictionary_key(&target) != normalized {
            return russian_entry(mdx, &target, Some(&lookup.key_text));
        }
    }
    russian_response(lookup.key_text, &lookup.definition, linked_from)
}

fn russian_exact_entries(mdx: &Mdx, target: &str) -> Vec<KeyWordItem> {
    let list = mdx.keyword_list();
    let mut lo = 0;
    let mut hi = list.len();
    while lo < hi {
        let mid = (lo + hi) / 2;
        if normalize_dictionary_key(&list[mid].key_text).as_str() < target {
            lo = mid + 1;
        } else {
            hi = mid;
        }
    }
    let start = lo;
    while lo < list.len() && normalize_dictionary_key(&list[lo].key_text) == target {
        lo += 1;
    }
    list[start..lo].to_vec()
}

fn russian_response(
    headword: String,
    definition: &str,
    linked_from: Option<&str>,
) -> Option<DictionaryEntryResponse> {
    let mut labels = vec!["OpenRussian".into()];
    if let Some(source) = linked_from {
        labels.push(format!("Lemma for {source}"));
    }
    Some(DictionaryEntryResponse {
        lemma: headword.clone(),
        matched_terms: linked_from.into_iter().map(str::to_string).collect(),
        headword,
        pronunciation: String::new(),
        definitions: Vec::new(),
        translations: Vec::new(),
        forms: Vec::new(),
        labels,
        examples: Vec::new(),
        detail: Value::Null,
        definition_html: definition.to_string(),
    })
}

fn russian_link_target(definition: &str) -> Option<String> {
    definition
        .split("@@@LINK=")
        .nth(1)?
        .split("@@@")
        .next()?
        .split_whitespace()
        .next()
        .map(|value| {
            value
                .trim_matches(|ch: char| matches!(ch, '"' | '\'' | '<' | '>'))
                .to_lowercase()
        })
}

fn german_database_path(directory: &std::path::Path) -> Result<PathBuf, ApiError> {
    let source = directory.join("german_en.sqlite.gz");
    let target = std::env::temp_dir().join("malim_chat_german_en.sqlite");
    if !target.exists() {
        let source_file = std::fs::File::open(source)
            .map_err(|_| ApiError::internal("German dictionary is not installed."))?;
        let mut decoder = GzDecoder::new(source_file);
        let mut output = std::fs::File::create(&target)
            .map_err(|_| ApiError::internal("German dictionary cache could not be created."))?;
        std::io::copy(&mut decoder, &mut output)
            .map_err(|_| ApiError::internal("German dictionary could not be unpacked."))?;
    }
    Ok(target)
}

fn lookup_german_dictionary(
    directory: &std::path::Path,
    word: &str,
) -> Result<Vec<DictionaryEntryResponse>, ApiError> {
    let connection = Connection::open(german_database_path(directory)?)
        .map_err(|_| ApiError::internal("German dictionary could not be opened."))?;
    let key = normalize_dictionary_key(word).replace('ß', "ss");
    let mut statement = connection.prepare("SELECT e.headword,e.lemma,e.forms_json,e.definition_html FROM german_lookup l JOIN german_entries e ON e.id=l.entry_id WHERE l.form_key=?1 LIMIT 12")
        .map_err(|_| ApiError::internal("German dictionary schema is invalid."))?;
    let rows = statement
        .query_map([key.clone()], |row| {
            let forms: String = row.get(2)?;
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                forms,
                row.get::<_, String>(3)?,
            ))
        })
        .map_err(|_| ApiError::internal("German dictionary query failed."))?;
    let mut entries = rows
        .filter_map(Result::ok)
        .map(
            |(headword, lemma, forms, definition)| DictionaryEntryResponse {
                lemma: lemma.clone(),
                matched_terms: vec![word.to_string()],
                headword,
                pronunciation: String::new(),
                definitions: Vec::new(),
                translations: Vec::new(),
                forms: serde_json::from_str::<Vec<String>>(&forms).unwrap_or_else(|_| vec![lemma]),
                labels: vec!["Kaikki German".into()],
                examples: Vec::new(),
                detail: Value::Null,
                definition_html: definition,
            },
        )
        .collect::<Vec<_>>();
    if entries.is_empty() {
        let mut fallback = connection.prepare("SELECT headword,lemma,forms_json,definition_html FROM german_entries WHERE headword_key LIKE ?1 LIMIT 12").map_err(|_| ApiError::internal("German dictionary query failed."))?;
        let rows = fallback
            .query_map([format!("{key}%")], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                ))
            })
            .map_err(|_| ApiError::internal("German dictionary query failed."))?;
        entries = rows
            .filter_map(Result::ok)
            .map(
                |(headword, lemma, forms, definition)| DictionaryEntryResponse {
                    lemma: lemma.clone(),
                    matched_terms: vec![word.to_string()],
                    headword,
                    pronunciation: String::new(),
                    definitions: Vec::new(),
                    translations: Vec::new(),
                    forms: serde_json::from_str(&forms).unwrap_or_else(|_| vec![lemma]),
                    labels: vec!["Kaikki German".into()],
                    examples: Vec::new(),
                    detail: Value::Null,
                    definition_html: definition,
                },
            )
            .collect();
    }
    Ok(entries)
}

fn lookup_english_chinese_dictionary(
    directory: &std::path::Path,
    word: &str,
) -> Result<Vec<DictionaryEntryResponse>, ApiError> {
    let connection = Connection::open(directory.join("ecdict_en_zh.sqlite"))
        .map_err(|_| ApiError::internal("English-Chinese dictionary could not be opened."))?;
    let normalized = word.trim().to_lowercase();
    let mut statement = connection.prepare("SELECT word,phonetic,definition,translation,pos,collins,oxford,tags,bnc,frequency,exchange,detail FROM entries WHERE word = ?1 COLLATE NOCASE UNION ALL SELECT word,phonetic,definition,translation,pos,collins,oxford,tags,bnc,frequency,exchange,detail FROM entries WHERE word LIKE ?2 COLLATE NOCASE LIMIT 12")
        .map_err(|_| ApiError::internal("English-Chinese dictionary schema is invalid."))?;
    let rows = statement
        .query_map(params![normalized, format!("{normalized}%")], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, i32>(5)?,
                row.get::<_, i32>(6)?,
                row.get::<_, String>(7)?,
                row.get::<_, i32>(8)?,
                row.get::<_, i32>(9)?,
                row.get::<_, String>(10)?,
                row.get::<_, String>(11)?,
            ))
        })
        .map_err(|_| ApiError::internal("English-Chinese dictionary query failed."))?;
    Ok(rows
        .filter_map(Result::ok)
        .map(
            |(
                headword,
                pronunciation,
                definition,
                translation,
                pos,
                collins,
                oxford,
                tags,
                bnc,
                frequency,
                exchange,
                detail,
            )| {
                let mut labels = split_lines(&tags);
                if !pos.is_empty() {
                    labels.push(format!("POS: {pos}"));
                }
                if collins > 0 {
                    labels.push(format!("Collins {collins}"));
                }
                if oxford > 0 {
                    labels.push("Oxford 3000".into());
                }
                if bnc > 0 {
                    labels.push(format!("BNC #{bnc}"));
                }
                if frequency > 0 {
                    labels.push(format!("Modern frequency #{frequency}"));
                }
                let forms = exchange
                    .split('/')
                    .filter_map(|item| item.split_once(':'))
                    .map(|(kind, value)| {
                        format!(
                            "{}: {}",
                            match kind {
                                "p" => "past",
                                "d" => "past participle",
                                "i" => "present participle",
                                "3" => "third-person singular",
                                "r" => "comparative",
                                "t" => "superlative",
                                "s" => "plural",
                                "0" => "lemma",
                                _ => kind,
                            },
                            value
                        )
                    })
                    .collect();
                let detail_value = serde_json::from_str(&detail).unwrap_or_else(|_| {
                    if detail.is_empty() {
                        Value::Null
                    } else {
                        Value::String(detail)
                    }
                });
                DictionaryEntryResponse {
                    lemma: headword.clone(),
                    matched_terms: vec![word.to_string()],
                    headword,
                    pronunciation,
                    definitions: split_lines(&definition),
                    translations: split_lines(&translation),
                    forms,
                    labels,
                    examples: Vec::new(),
                    detail: detail_value,
                    definition_html: String::new(),
                }
            },
        )
        .collect())
}
