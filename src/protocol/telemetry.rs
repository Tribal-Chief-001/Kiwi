use crate::metrics::EdgeTelemetry;
use http::HeaderMap;

pub fn parse_edge_telemetry(headers: &HeaderMap) -> EdgeTelemetry {
    let mut telemetry = EdgeTelemetry::default();

    // Parse Colo
    if let Some(val) = headers.get("colo").or_else(|| headers.get("cf-meta-colo"))
        && let Ok(s) = val.to_str()
    {
        telemetry.colo = s.trim().to_string();
    }

    // Parse ASN
    if let Some(val) = headers.get("asn").or_else(|| headers.get("cf-meta-asn"))
        && let Ok(s) = val.to_str()
    {
        telemetry.asn = s.trim().parse().ok();
    }

    // Parse City
    if let Some(val) = headers.get("city").or_else(|| headers.get("cf-meta-city"))
        && let Ok(s) = val.to_str()
    {
        telemetry.city = s.trim().to_string();
    }

    // Parse Country
    if let Some(val) = headers
        .get("country")
        .or_else(|| headers.get("cf-meta-country"))
        && let Ok(s) = val.to_str()
    {
        telemetry.country = s.trim().to_string();
    }

    // Parse Client IP
    if let Some(val) = headers.get("cf-meta-ip")
        && let Ok(s) = val.to_str()
    {
        telemetry.client_ip = s.trim().to_string();
    }

    // Parse server-timing: cfL4;desc="..."
    for val in headers.get_all("server-timing") {
        if let Ok(s) = val.to_str()
            && let Some(cfl4_pos) = s.find("cfL4")
        {
            let rest = &s[cfl4_pos..];
            if let Some(desc_start) = rest.find("desc=\"") {
                let desc_content = &rest[desc_start + 6..];
                if let Some(desc_end) = desc_content.find('"') {
                    let query = &desc_content[..desc_end];
                    parse_cfl4_query(query, &mut telemetry);
                }
            }
        }
    }

    telemetry
}

fn parse_cfl4_query(query: &str, telemetry: &mut EdgeTelemetry) {
    let trimmed = query.trim_start_matches('?');
    for pair in trimmed.split('&') {
        let mut parts = pair.splitn(2, '=');
        let key = parts.next().unwrap_or("").trim();
        let val = parts.next().unwrap_or("").trim();

        match key {
            "proto" => telemetry.cf_l4_proto = Some(val.to_string()),
            "rtt" => telemetry.cf_l4_rtt_us = val.parse().ok(),
            "min_rtt" => telemetry.cf_l4_min_rtt_us = val.parse().ok(),
            "cwnd" => telemetry.cf_l4_cwnd = val.parse().ok(),
            "retrans" => telemetry.cf_l4_retrans = val.parse().ok(),
            "delivery_rate" => telemetry.cf_l4_delivery_rate = val.parse().ok(),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use http::HeaderValue;

    #[test]
    fn test_parse_cfl4() {
        let mut headers = HeaderMap::new();
        headers.insert("colo", HeaderValue::from_static("BOM"));
        headers.insert("asn", HeaderValue::from_static("55836"));
        headers.insert("city", HeaderValue::from_static("Mumbai"));
        headers.insert("country", HeaderValue::from_static("IN"));
        headers.insert(
            "cf-meta-ip",
            HeaderValue::from_static("2409:40c4:19:8727::1"),
        );
        headers.insert(
            "server-timing",
            HeaderValue::from_static(
                r#"cfL4;desc="?proto=TCP&rtt=42899&min_rtt=39859&cwnd=53&retrans=0&delivery_rate=73129""#,
            ),
        );

        let telem = parse_edge_telemetry(&headers);
        assert_eq!(telem.colo, "BOM");
        assert_eq!(telem.asn, Some(55836));
        assert_eq!(telem.city, "Mumbai");
        assert_eq!(telem.country, "IN");
        assert_eq!(telem.cf_l4_proto.as_deref(), Some("TCP"));
        assert_eq!(telem.cf_l4_rtt_us, Some(42899));
        assert_eq!(telem.cf_l4_min_rtt_us, Some(39859));
        assert_eq!(telem.cf_l4_cwnd, Some(53));
        assert_eq!(telem.cf_l4_retrans, Some(0));
        assert_eq!(telem.cf_l4_delivery_rate, Some(73129));
    }
}
