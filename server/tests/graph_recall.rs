//! The assistant asks the graph (#46): an organization by any name it goes by, people by the role they have in a
//! matter, matters by how they stand ("견적은 나갔는데 계약은 안 된"), requests not answered. What the graph finds comes
//! first, then what the words find; a question without anything the graph knows is searched as before.
mod common;

use std::fs;
use std::sync::Arc;
use std::time::Duration;

use common::FakeModel;
use life_server::assistant::recall::{self, Named, Picked};
use life_server::config::Paths;
use life_server::librarian::Librarian;
use life_server::librarian::graph;
use serde_json::{Value, json};

struct T {
    _dir: tempfile::TempDir,
    p: Paths,
}

const ME: &str = "나 <me@mine.co.kr>";

impl T {
    fn new() -> T {
        let dir = tempfile::tempdir().unwrap();
        let p = Paths::under(dir.path());
        fs::create_dir_all(p.vault.join("Events")).unwrap();
        T { _dir: dir, p }
    }

    #[allow(clippy::too_many_arguments)]
    fn mail(&self, name: &str, day: &str, from: &str, to: &str, subject: &str, body: &str, sent: bool, refs: &str) {
        let path = self.p.vault.join("Mail").join(&day[..4]).join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let source = if sent { "account/Sent Messages" } else { "account/INBOX" };
        fs::write(
            path,
            format!("---\ntype: mail\nmail_id: <{name}@x>\nmail_source: {source}\ndate: {day}\ntime: 09:00\nfrom: {from}\nto: [{to}]\ncc: []\nsubject: \"{subject}\"\nreferences: {refs}\n---\n## Message\n{body}\n"),
        )
        .unwrap();
    }
}

/// The model's reading of each mail, by a word in it.
fn reader() -> Arc<FakeModel> {
    FakeModel::new(|msgs| {
        let text = msgs.last().unwrap()["content"].as_str().unwrap_or("").to_string();
        let r = if text.contains("견적서 송부") {
            json!([["설비 점검:case", "state", "2026-03-02", "견적 발송", "2026-03-02", "견적서 송부"]])
        } else if text.contains("계약 체결") {
            json!([["설비 점검:case", "state", "2026-03-10", "계약", "2026-03-10", "계약 체결"]])
        } else if text.contains("배관 견적") {
            json!([["배관 교체:case", "state", "2026-03-05", "견적서 발송", "2026-03-05", "배관 견적"]])
        } else if text.contains("포트폴리오") {
            json!([
                ["정다운:person", "role", "연구원 채용:case", "지원자", "2026-04-10", "지원자 정다운"],
                ["인사팀:org", "sent", "포트폴리오:doc", "포트폴리오", "2026-04-10", "포트폴리오 송부"]
            ])
        } else if text.contains("자료 요청") {
            json!([[
                "박고객 <park@client.co.kr>:person",
                "requested",
                "측정 자료:doc",
                "측정 자료",
                "2026-03-20",
                "자료 요청"
            ]])
        } else if text.contains("설악이앤씨") {
            json!([["(주)설악이앤씨:org", "role", "센서 결선:case", "공급", "2026-05-01", "(주)설악이앤씨"]])
        } else if text.contains("지원사업") {
            json!([["제조 AI 지원사업:case", "about", "공고문:doc", "", "2026-09-29", "지원사업"]])
        } else {
            json!([])
        };
        Ok(json!({"r": r}).to_string())
    })
}

fn world() -> (T, Librarian) {
    let t = T::new();
    // an organization the mails name only in their subjects
    t.mail(
        "k1.md",
        "2026-10-21",
        "담당자 <a@ksa.or.kr>",
        ME,
        "[한국표준협회] 1차 워크숍 계획",
        "워크숍 일정 공유",
        false,
        "[]",
    );
    t.mail("k2.md", "2026-10-22", "다른 담당 <b@ksa.or.kr>", ME, "개발 회의 줌 링크", "회의 링크입니다", false, "[]");
    t.mail(
        "k3.md",
        "2026-10-23",
        ME,
        "담당자 <a@ksa.or.kr>",
        "RE: [한국표준협회] 1차 워크숍 계획",
        "참석합니다",
        true,
        "[<k1.md@x>]",
    );
    // matters by how they stand
    t.mail("q1.md", "2026-03-02", ME, "고객 <c@client.co.kr>", "설비 점검 견적", "견적서 송부드립니다", true, "[]");
    t.mail(
        "q2.md",
        "2026-03-10",
        "고객 <c@client.co.kr>",
        ME,
        "RE: 설비 점검 견적",
        "계약 체결 진행하겠습니다",
        false,
        "[<q1.md@x>]",
    );
    t.mail("q3.md", "2026-03-05", ME, "다른고객 <d@other.co.kr>", "배관 교체 견적", "배관 견적 보냅니다", true, "[]");
    // a person known by a role, in a mail someone else sent
    t.mail(
        "h1.md",
        "2026-04-10",
        "인사팀 <hr@mine-hr.co.kr>",
        ME,
        "[2차 면접 자료] 지원자 정다운 포트폴리오",
        "포트폴리오 송부드립니다",
        false,
        "[]",
    );
    // requests: one answered, one not
    t.mail(
        "r1.md",
        "2026-03-20",
        "박고객 <park@client.co.kr>",
        ME,
        "측정 자료 요청",
        "측정 자료 요청드립니다",
        false,
        "[]",
    );
    t.mail(
        "r2.md",
        "2026-03-21",
        "박고객 <park@client.co.kr>",
        ME,
        "설치 사진 요청",
        "사진 자료 요청드립니다",
        false,
        "[]",
    );
    t.mail(
        "r3.md",
        "2026-03-22",
        ME,
        "박고객 <park@client.co.kr>",
        "RE: 측정 자료 요청",
        "보내드립니다",
        true,
        "[<r1.md@x>]",
    );
    t.mail(
        "s1.md",
        "2026-09-29",
        "공고 <n@trust.co.kr>",
        ME,
        "제조 AI 현장 적용 지원사업 모집 공고",
        "지원사업 안내",
        false,
        "[]",
    );
    let mut lib = Librarian::open(&t.p, None, Duration::from_secs(5)).unwrap();
    lib.update(false).unwrap();
    graph::extract(&mut lib, reader().as_ref(), &graph::Run { limit: 50, workers: 2 }).unwrap();
    (t, lib)
}

fn titles(hits: &[Value]) -> Vec<String> {
    hits.iter().map(|h| h["title"].as_str().unwrap_or("").trim_start_matches("RE: ").to_string()).collect()
}

fn ask(lib: &mut Librarian, p: Picked) -> Vec<Value> {
    match recall::lookup(lib, &p, 8, "2026-10-31", &[]).unwrap() {
        Named::Hits(h) => h,
        Named::Ask(_) => panic!("asked back"),
    }
}

#[test]
fn an_organization_named_only_in_subjects_is_found_by_its_mail_domain_with_all_its_matters() {
    let (_t, mut lib) = world();
    let hits = ask(&mut lib, Picked { orgs: vec!["한국표준협회".into()], ..Picked::default() });
    let mut got = titles(&hits);
    got.sort();
    assert_eq!(got[..2], ["[한국표준협회] 1차 워크숍 계획".to_string(), "개발 회의 줌 링크".to_string()], "{got:?}");
    assert!(!got.iter().any(|t| t.contains("견적")), "{got:?}");
}

#[test]
fn a_person_with_a_role_is_found_in_what_someone_else_sent_about_them() {
    let (_t, mut lib) = world();
    let hits =
        ask(&mut lib, Picked { people: vec!["정다운".into()], roles: vec!["지원자".into()], ..Picked::default() });
    assert_eq!(titles(&hits).first().map(String::as_str), Some("[2차 면접 자료] 지원자 정다운 포트폴리오"));
}

#[test]
fn matters_are_found_by_how_they_stand_and_what_did_not_follow() {
    let (_t, mut lib) = world();
    let hits = ask(&mut lib, Picked { states: vec!["견적".into()], absent: vec!["계약".into()], ..Picked::default() });
    let got = titles(&hits);
    assert_eq!(got.first().map(String::as_str), Some("배관 교체 견적"), "{got:?}");
    assert!(!got.iter().take(1).any(|t| t.contains("설비")), "a quote followed by a contract is not one: {got:?}");
    assert!(hits[0]["state"].as_str().unwrap().contains("견적서 발송"), "{:?}", hits[0]);
}

#[test]
fn a_request_with_no_mail_of_the_users_after_it_is_open() {
    let (_t, mut lib) = world();
    let hits = ask(&mut lib, Picked { open_requests: true, ..Picked::default() });
    assert_eq!(titles(&hits), ["설치 사진 요청"], "the answered one is not open");
}

#[test]
fn the_graph_and_the_words_take_turns_and_a_plain_question_is_searched_as_before() {
    let (_t, mut lib) = world();
    let hits =
        ask(&mut lib, Picked { things: vec!["지원사업".into()], words: vec!["워크숍".into()], ..Picked::default() });
    let got = titles(&hits);
    let mut top: Vec<String> = got[..2].to_vec();
    top.sort();
    assert_eq!(
        top,
        ["[한국표준협회] 1차 워크숍 계획".to_string(), "제조 AI 현장 적용 지원사업 모집 공고".to_string()],
        "what the graph found and what the words found, both at the top: {got:?}"
    );
    let n = got.iter().filter(|t| t.contains("워크숍")).count();
    assert_eq!(n, 1, "one matter is one hit: {got:?}");
    let plain = ask(&mut lib, Picked { words: vec!["배관".into()], ..Picked::default() });
    assert_eq!(titles(&plain), titles(&lib.search("배관", 8, None).unwrap()));
}

#[test]
fn the_words_picked_for_the_graph_are_read_from_the_model() {
    let m = FakeModel::new(|_| {
        Ok(r#"{"search": true, "people": [], "words": ["견적"], "orgs": ["한국표준협회"], "things": ["FMS"],
               "roles": ["담당"], "states": ["견적"], "absent": ["계약"], "open_requests": true}"#
            .into())
    });
    let p = recall::pick(m.as_ref(), "한국표준협회 쪽 견적만 나간 거", None).unwrap();
    assert_eq!(
        (p.orgs, p.things, p.roles, p.states, p.absent, p.open_requests),
        (
            vec!["한국표준협회".to_string()],
            vec!["FMS".to_string()],
            vec!["담당".to_string()],
            vec!["견적".to_string()],
            vec!["계약".to_string()],
            true
        )
    );
}

// ---------- abbreviations and spellings ----------

#[test]
fn an_organization_is_found_by_its_domain_by_a_guessed_short_name_and_by_another_spelling() {
    let (t, mut lib) = world();
    t.mail("x1.md", "2026-05-01", "담당 <a@seolakenc.com>", ME, "센서 결선도", "(주)설악이앤씨 입니다", false, "[]");
    lib.update(false).unwrap();
    graph::extract(&mut lib, reader().as_ref(), &graph::Run { limit: 50, workers: 1 }).unwrap();
    let orgs = |lib: &mut Librarian, name: &str| -> Vec<String> {
        titles(&ask(lib, Picked { orgs: vec![name.into()], ..Picked::default() }))
    };
    assert!(orgs(&mut lib, "KSA").contains(&"[한국표준협회] 1차 워크숍 계획".to_string()), "the domain is a name");
    let guess = FakeModel::new(|_| {
        Ok(r#"{"a": {"org:ksa.or.kr": ["표준협회", "KSA"], "org:seolakenc.com": ["설악ENC"]}}"#.into())
    });
    graph::abbreviate_orgs(&lib, guess.as_ref()).unwrap();
    assert!(orgs(&mut lib, "표준협회").contains(&"[한국표준협회] 1차 워크숍 계획".to_string()), "a short name guessed");
    assert!(orgs(&mut lib, "설악ENC").contains(&"센서 결선도".to_string()));
    assert!(orgs(&mut lib, "설악이엔씨").contains(&"센서 결선도".to_string()), "ㅐ and ㅔ alike");
    assert!(
        lib.node("org:ksa.or.kr").unwrap().aliases.iter().all(|a| a != "표준협회"),
        "a guess is no fact of the records"
    );
}

#[test]
fn a_company_named_in_a_colleagues_mail_is_not_the_users_own_company() {
    let (t, mut lib) = world();
    t.mail(
        "c1.md",
        "2026-05-02",
        "동료 <col@mine.co.kr>",
        ME,
        "협력사 소개",
        "다른상사 담당 홍길동 소개드립니다",
        false,
        "[]",
    );
    t.mail("c2.md", "2026-05-03", "동료 <col@mine.co.kr>", ME, "점심 메뉴", "오늘 점심", false, "[]");
    lib.update(false).unwrap();
    let m = FakeModel::new(|msgs| {
        let text = msgs.last().unwrap()["content"].as_str().unwrap_or("");
        let r = if text.contains("다른상사") {
            json!([["홍길동:person", "member", "다른상사:org", "", "2026-05-02", "다른상사 담당 홍길동"]])
        } else {
            json!([])
        };
        Ok(json!({"r": r}).to_string())
    });
    graph::extract(&mut lib, m.as_ref(), &graph::Run { limit: 50, workers: 1 }).unwrap();
    let got = titles(&ask(&mut lib, Picked { orgs: vec!["다른상사".into()], ..Picked::default() }));
    assert!(!got.contains(&"점심 메뉴".to_string()), "the user's own company is not the one named: {got:?}");
}

#[test]
fn a_name_the_picker_took_for_a_person_or_a_word_is_tried_as_an_organization_when_it_is_exactly_one() {
    let (t, mut lib) = world();
    t.mail("m1.md", "2026-06-01", "영업 <s@monnit.com>", ME, "센서 단가", "단가표입니다", false, "[]");
    lib.update(false).unwrap();
    let guess = FakeModel::new(|_| Ok(r#"{"a": {"org:monnit.com": ["모닛"]}}"#.into()));
    graph::abbreviate_orgs(&lib, guess.as_ref()).unwrap();
    for p in [
        Picked { people: vec!["모닛".into()], ..Picked::default() },
        Picked { words: vec!["모닛".into()], ..Picked::default() },
    ] {
        assert!(titles(&ask(&mut lib, p)).contains(&"센서 단가".to_string()));
    }
    // a word that is only part of an organization's name is a word
    let p = Picked { words: vec!["센서".into()], ..Picked::default() };
    assert_eq!(titles(&ask(&mut lib, p)), titles(&lib.search("센서", 8, None).unwrap()));
}

#[test]
fn an_organization_is_found_by_how_its_domain_sounds_in_korean_when_no_name_of_it_fits() {
    let (t, mut lib) = world();
    t.mail("d1.md", "2026-06-02", "고대원 <ben@dekist.com>", ME, "HTTP 헤더 문의", "헤더 값 확인 부탁", false, "[]");
    t.mail("w1.md", "2026-06-03", "알림 <noreply@wehago.com>", ME, "전자세금계산서 발행", "발행 안내", false, "[]");
    lib.update(false).unwrap();
    for p in [
        Picked { people: vec!["디키스트".into()], ..Picked::default() },
        Picked { orgs: vec!["디키스트".into()], ..Picked::default() },
    ] {
        assert!(titles(&ask(&mut lib, p)).contains(&"HTTP 헤더 문의".to_string()), "디키스트 is dekist");
    }
    let p = Picked { words: vec!["위하고".into()], ..Picked::default() };
    assert!(titles(&ask(&mut lib, p)).contains(&"전자세금계산서 발행".to_string()), "위하고 is wehago");
    // a common word that only sounds like no domain stays a word
    let p = Picked { words: vec!["헤더".into()], ..Picked::default() };
    assert_eq!(titles(&ask(&mut lib, p)), titles(&lib.search("헤더", 8, None).unwrap()));
}

#[test]
fn the_domains_that_carry_an_organizations_name_are_asked_about_with_it() {
    let (t, mut lib) = world();
    t.mail("a1.md", "2026-07-01", "영업 <s@kumulo.com>", ME, "플랫폼 견적", "견적 회신", false, "[]");
    t.mail(
        "a2.md",
        "2026-07-02",
        "지라 <jira@kumulo.atlassian.net>",
        ME,
        "CST-1 패치 가이드",
        "티켓 갱신",
        false,
        "[]",
    );
    t.mail(
        "a3.md",
        "2026-07-03",
        "지라 <jira@other.atlassian.net>",
        ME,
        "OT-9 다른 회사 티켓",
        "티켓 갱신",
        false,
        "[]",
    );
    lib.update(false).unwrap();
    let got = titles(&ask(&mut lib, Picked { orgs: vec!["쿠물로".into()], ..Picked::default() }));
    assert!(got.contains(&"플랫폼 견적".to_string()), "{got:?}");
    assert!(got.contains(&"CST-1 패치 가이드".to_string()), "its tickets come from kumulo.atlassian.net: {got:?}");
    assert!(!got.contains(&"OT-9 다른 회사 티켓".to_string()), "not every atlassian.net: {got:?}");
}

#[test]
fn a_korean_name_sounds_like_a_domain_only_when_its_consonants_and_vowels_go_along() {
    for (name, word) in
        [("디키스트", "dekist"), ("맥스테드", "maxted"), ("위하고", "wehago"), ("큐물로시티", "cumulocity")]
    {
        assert!(graph::sounds_like(name, word), "{name} ~ {word}");
    }
    // common words a few consonants away from a domain, or with other vowels, are not it
    for (name, word) in
        [("오프라인", "fairino"), ("진천", "junjin"), ("미츠비시", "machbase"), ("공고", "konkuk"), ("헤더", "hd")]
    {
        assert!(!graph::sounds_like(name, word), "{name} !~ {word}");
    }
}
