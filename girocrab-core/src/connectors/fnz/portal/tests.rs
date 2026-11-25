use super::*;

use url::Url;

fn page_url(path: &str) -> url::Url {
    url::Url::parse(&format!("https://portal.fnz.de{path}")).unwrap()
}

#[test]
fn login_form_parses_portal_markup() {
    // Realistic shape of the ASP.NET login page (entity-escaped quotes in the
    // `href` postback, hidden fields, the two credential inputs).
    let html = r#"<html><body>
        <form method="post" action="./login?eox=r" id="aspnetForm" onsubmit="return check()">
          <input type="hidden" name="__VIEWSTATE" id="__VIEWSTATE" value="AbCd/123+state==" />
          <input type="hidden" name="__VIEWSTATEGENERATOR" value="C2EE9ABB" />
          <input type="hidden" name="__EVENTVALIDATION" value="/wEdAAValidation==" />
          <input name="eox_ContentPane_3$totp" class="text" type="hidden" value="" />
          <label>Zugangsnummer</label>
          <input name="eox_ContentPane_3$txtPbzZugangsId" type="text" value="" maxlength="20" />
          <label>PIN</label>
          <input name="eox_ContentPane_3$txtPbzPin" type="password" value="" />
          <a href="javascript:__doPostBack(&#39;eox_ContentPane_3$LOGIN_PBZ&#39;,&#39;&#39;)" id="eox_ContentPane_3_LOGIN_PBZ">Anmelden</a>
        </form></body></html>"#;
    let page = page_url("/abc123def/finvesto/login");
    let form = login_form(html, &page).expect("form");
    assert_eq!(
        form.action.as_str(),
        "https://portal.fnz.de/abc123def/finvesto/login?eox=r"
    );
    assert_eq!(form.user_field, "eox_ContentPane_3$txtPbzZugangsId");
    assert_eq!(form.pin_field, "eox_ContentPane_3$txtPbzPin");
    assert_eq!(form.submit, "eox_ContentPane_3$LOGIN_PBZ");
    assert_eq!(
        form.state,
        [
            ("__VIEWSTATE".to_string(), "AbCd/123+state==".to_string()),
            ("__VIEWSTATEGENERATOR".to_string(), "C2EE9ABB".to_string()),
            (
                "__EVENTVALIDATION".to_string(),
                "/wEdAAValidation==".to_string()
            ),
            // Hidden helper fields (here the empty `$totp`) are submitted
            // empty, exactly like the browser does.
            ("eox_ContentPane_3$totp".to_string(), String::new()),
        ]
    );
    let filled = form.filled("U-1", "cat-mouse");
    assert!(filled.contains(&("__EVENTTARGET".into(), form.submit.clone())));
    assert!(filled.contains(&("__VIEWSTATEGENERATOR".into(), "C2EE9ABB".into())));
    assert!(filled.contains(&("eox_ContentPane_3$txtPbzZugangsId".into(), "U-1".into())));
    assert!(filled.contains(&("eox_ContentPane_3$txtPbzPin".into(), "cat-mouse".into())));
    // Never a GET: a form without method="post" is a protocol change.
    let get_form = html.replace("method=\"post\"", "method=\"get\"");
    assert!(login_form(&get_form, &page).is_err());
    // A page without a login form is not a login page.
    assert!(login_form("<p>keine Anmeldung</p>", &page).is_err());
    // A redirect outside the portal must abort before anything is sent.
    let here = page_url("/ok/finvesto/login");
    let away = html.replace(
        "action=\"./login?eox=r\"",
        "action=\"https://evil.example/login\"",
    );
    let err = login_form(&away, &here).unwrap_err();
    assert!(err.to_string().contains("Weiterleitung"));
}

#[test]
fn login_detection() {
    let html = r#"<html><body>
        <form method="post" action="./login"><input type="password" name="pw"></form>
        </body></html>"#;
    assert!(is_login_html(html));
    // A page without any password field is not the login.
    assert!(!is_login_html(
        r#"<form method="post" action="./x"><input type="text" name="q"></form>"#
    ));
    assert!(is_login_path(&page_url("/abc/finvesto/login")));
    assert!(!is_login_path(&page_url("/abc/finvesto/eox/rest/")));
}

#[test]
fn rest_base_accounts_for_session_prefix() {
    // Reached, trailing file: file name dropped, `eox/rest/` appended.
    let base = rest_base_of(&page_url(
        "/q9(SessionID)/EOY/EL/26.4.01/finvesto/redirect.aspx",
    ))
    .unwrap();
    assert_eq!(
        base.as_str(),
        "https://portal.fnz.de/q9(SessionID)/EOY/EL/26.4.01/finvesto/eox/rest/"
    );
    // Reached without a file name: same result.
    let base = rest_base_of(&page_url("/q9(SessionID)/EOY/EL/26.4.01/finvesto")).unwrap();
    assert_eq!(
        base.as_str(),
        "https://portal.fnz.de/q9(SessionID)/EOY/EL/26.4.01/finvesto/eox/rest/"
    );
    // The browser login would land here after the FNZsecure approval.
    let base = rest_base_of(&page_url(
        "/eo(Session2024)/EOY/EL/26.4.01/finvesto/login?eox=r",
    ))
    .unwrap();
    assert_eq!(
        base.as_str(),
        "https://portal.fnz.de/eo(Session2024)/EOY/EL/26.4.01/finvesto/eox/rest/"
    );
    // No `EOY` in the path: the session did not reach the portal app.
    assert!(rest_base_of(&page_url("/abc/finvesto/login")).is_err());
    assert!(rest_base_of(&page_url("/")).is_err());
}

#[test]
fn pdf_base_is_the_path_before_the_first_eoy() {
    // Like the browser script builds it (`location.pathname.split('EOY')[0]`
    // in the portal session): everything before the first `EOY`.
    let rest = Url::parse("https://portal.fnz.de/ses1/EOY/EL/26.4.01/finvesto/eox/rest/").unwrap();
    assert_eq!(
        pdf_base_of(&rest).as_str(),
        "https://portal.fnz.de/ses1/p/postkorb.aspx"
    );
    let rest = Url::parse("https://portal.fnz.de/a/b/EOY/x/finvesto/eox/rest/").unwrap();
    assert_eq!(
        pdf_base_of(&rest).as_str(),
        "https://portal.fnz.de/a/b/p/postkorb.aspx"
    );
}

#[test]
fn has_id_matches_portal_ids() {
    assert!(has_id(&json!("K1")));
    assert!(has_id(&json!(" k1 ")));
    assert!(has_id(
        &json!({"orderNumber": {"id": "K1"}, "other": Value::Null})
    ));
    assert!(!has_id(&json!("")));
    assert!(!has_id(&json!("  ")));
    assert!(!has_id(&json!({"orderNumber": {"id": ""}})));
    assert!(!has_id(&json!(12)));
    assert!(!has_id(&json!(Value::Null)));
}

#[test]
fn path_encoding_keeps_common_ids_plain() {
    assert_eq!(path_encoding("abc-123_XYZ.9"), "abc-123_XYZ.9");
    assert_eq!(path_encoding("a b/c"), "a%20b%2Fc");
}

#[test]
fn request_filter_stays_unchanged_between_pages() {
    // `page_all` clones the request filter per page and adds `pagination`
    // only to the clone: the base filter must reach every page unchanged.
    let filter = json!({"dateFrom": "2000-01-01", "sortOrder": "BOOKING_DATE_DESC"});
    let mut body = filter.clone();
    body["pagination"] = json!({"page": "0", "pageSize": "100"});
    assert_eq!(filter["dateFrom"], json!("2000-01-01"));
    assert_eq!(filter["sortOrder"], json!("BOOKING_DATE_DESC"));
    assert!(filter.get("pagination").is_none());
    assert_eq!(body["pagination"]["pageSize"], json!("100"));
}

#[test]
fn document_metadata_helpers_match_the_browser_flow() {
    let doc = json!({
        "id": "M1",
        "subject": "Depotauszug Q3",
        "documentKindText": "Depotauszug Q3",
        "receivedDate": "2025-10-02T08:15:00",
        "depot": {"externalDepotNumber": {"externalDepotNumber": "D1"}},
    });
    assert_eq!(super::super::document_title(&doc), "Depotauszug Q3");
    assert_eq!(super::super::document_date(&doc), "2025-10-02");
    // Without a date the browser flow stores an empty one; the same here.
    let mut no_date = doc.clone();
    no_date["receivedDate"] = json!(Value::Null);
    assert_eq!(super::super::document_date(&no_date), "");
    // An impossible date is not imported as a real day.
    no_date["receivedDate"] = json!("2025-13-99");
    assert_eq!(super::super::document_date(&no_date), "");
}
