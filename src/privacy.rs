use std::net::IpAddr;

use serde::{Deserialize, Serialize};
use url::Url;

use crate::{AmosError, Result};

pub const CUSTOMER_CONTROLLED_DEPLOYMENT: &str = "customer-controlled single host";
pub const LOCAL_READ_ONLY_DATA_EXECUTION: &str = "local read-only warehouse";
pub const HOSTED_GEMMA_ROUTE: &str = "hosted Gemma API — approved egress";
pub const EXTERNAL_TELEMETRY_DISABLED: &str = "disabled";
pub const GOVERNED_HOSTED_TAGLINE: &str =
    "Customer-controlled, auditable AI analyst with governed model access";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClaimContract {
    pub claim: &'static str,
    pub required_visual_proof: &'static str,
    pub acceptance_condition: &'static str,
}

pub const CLAIM_CONTRACT: [ClaimContract; 4] = [
    ClaimContract {
        claim: "Private",
        required_visual_proof: "Deployment boundary, model route, telemetry status, and permission-filtered payload",
        acceptance_condition: "Local execution is distinguished from approved egress; credentials and raw rows are excluded",
    },
    ClaimContract {
        claim: "AI analyst",
        required_visual_proof: "Live model identity, typed plan, and model-authored narrative plan",
        acceptance_condition: "No deterministic planner or captured response is substituted in the filmed run",
    },
    ClaimContract {
        claim: "Auditable",
        required_visual_proof: "Claim evidence, review decision, source versions, model hashes, and source-change impact",
        acceptance_condition: "Every material claim resolves to durable evidence across refresh and restart",
    },
    ClaimContract {
        claim: "AMOS as layer",
        required_visual_proof: "Plan admission, policy checks, narrow capability, external execution, verification, and publication gate",
        acceptance_condition: "The model receives no warehouse handle or credential and cannot execute or publish",
    },
];

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ModelRouteClass {
    Local,
    CustomerVpcPrivateEndpoint,
    ApprovedHostedApi,
}

impl ModelRouteClass {
    pub const fn display_name(self) -> &'static str {
        match self {
            Self::Local => "local model endpoint — no public model egress",
            Self::CustomerVpcPrivateEndpoint => "customer VPC private model endpoint",
            Self::ApprovedHostedApi => HOSTED_GEMMA_ROUTE,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PrivacyProfile {
    AirGapped,
    ApprovedApi,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PrivacyBoundaryConfig {
    pub profile: PrivacyProfile,
    pub model_route_class: ModelRouteClass,
    pub model_base_url: String,
    pub allowed_egress_hosts: Vec<String>,
    pub external_telemetry: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PrivacyBoundaryView {
    pub deployment: &'static str,
    pub data_execution: &'static str,
    pub model_route: &'static str,
    pub privacy_profile: PrivacyProfile,
    pub public_egress_allowlist: Vec<String>,
    pub external_telemetry: &'static str,
    pub qualified_product_claim: &'static str,
}

impl PrivacyBoundaryConfig {
    pub fn local_air_gapped() -> Self {
        Self {
            profile: PrivacyProfile::AirGapped,
            model_route_class: ModelRouteClass::Local,
            model_base_url: "http://127.0.0.1".into(),
            allowed_egress_hosts: Vec::new(),
            external_telemetry: false,
        }
    }

    pub fn validate(&self) -> Result<()> {
        let url = Url::parse(&self.model_base_url)
            .map_err(|_| AmosError::Validation("model base URL is invalid".into()))?;
        if !matches!(url.scheme(), "http" | "https") {
            return Err(AmosError::Validation(
                "model base URL must use HTTP or HTTPS".into(),
            ));
        }
        let host = url
            .host_str()
            .ok_or_else(|| AmosError::Validation("model base URL must include a host".into()))?;
        let public_endpoint = is_public_host(host);

        if self.profile == PrivacyProfile::AirGapped && public_endpoint {
            return Err(AmosError::Validation(
                "air_gapped privacy profile rejects public model endpoints".into(),
            ));
        }
        if self.profile == PrivacyProfile::AirGapped
            && self.model_route_class == ModelRouteClass::ApprovedHostedApi
        {
            return Err(AmosError::Validation(
                "air_gapped privacy profile rejects approved hosted API routes".into(),
            ));
        }
        if self.model_route_class == ModelRouteClass::ApprovedHostedApi {
            if self.profile != PrivacyProfile::ApprovedApi {
                return Err(AmosError::Validation(
                    "hosted API routes require the approved_api privacy profile".into(),
                ));
            }
            if !self
                .allowed_egress_hosts
                .iter()
                .any(|allowed| allowed.eq_ignore_ascii_case(host))
            {
                return Err(AmosError::Validation(
                    "model endpoint host is absent from the egress allowlist".into(),
                ));
            }
        }
        if self.model_route_class == ModelRouteClass::Local && public_endpoint {
            return Err(AmosError::Validation(
                "local model route class cannot name a public endpoint".into(),
            ));
        }
        if self.model_route_class == ModelRouteClass::CustomerVpcPrivateEndpoint && public_endpoint
        {
            return Err(AmosError::Validation(
                "customer VPC private route class cannot name a public endpoint".into(),
            ));
        }
        if self.profile == PrivacyProfile::AirGapped && !self.allowed_egress_hosts.is_empty() {
            return Err(AmosError::Validation(
                "air_gapped privacy profile cannot configure public egress hosts".into(),
            ));
        }
        Ok(())
    }

    pub fn view(&self) -> Result<PrivacyBoundaryView> {
        self.validate()?;
        Ok(PrivacyBoundaryView {
            deployment: CUSTOMER_CONTROLLED_DEPLOYMENT,
            data_execution: LOCAL_READ_ONLY_DATA_EXECUTION,
            model_route: self.model_route_class.display_name(),
            privacy_profile: self.profile,
            public_egress_allowlist: self.allowed_egress_hosts.clone(),
            external_telemetry: if self.external_telemetry {
                "enabled"
            } else {
                EXTERNAL_TELEMETRY_DISABLED
            },
            qualified_product_claim: if self.model_route_class == ModelRouteClass::ApprovedHostedApi
            {
                GOVERNED_HOSTED_TAGLINE
            } else {
                "Private, auditable AI analyst"
            },
        })
    }
}

fn is_public_host(host: &str) -> bool {
    if host.eq_ignore_ascii_case("localhost") || host.ends_with(".localhost") {
        return false;
    }
    match host.parse::<IpAddr>() {
        Ok(IpAddr::V4(address)) => {
            !(address.is_loopback()
                || address.is_private()
                || address.is_link_local()
                || address.is_unspecified())
        }
        Ok(IpAddr::V6(address)) => {
            !(address.is_loopback() || address.is_unique_local() || address.is_unspecified())
        }
        Err(_) => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn public_model_url_is_rejected_under_air_gapped() {
        let config = PrivacyBoundaryConfig {
            profile: PrivacyProfile::AirGapped,
            model_route_class: ModelRouteClass::ApprovedHostedApi,
            model_base_url: "https://generativelanguage.googleapis.com/v1beta".into(),
            allowed_egress_hosts: vec!["generativelanguage.googleapis.com".into()],
            external_telemetry: false,
        };

        let error = config
            .validate()
            .expect_err("public route must fail closed");
        assert!(error.to_string().contains("air_gapped"));
        assert!(!error.to_string().contains("generativelanguage"));
    }

    #[test]
    fn hosted_route_view_uses_qualified_claim_vocabulary() {
        let config = PrivacyBoundaryConfig {
            profile: PrivacyProfile::ApprovedApi,
            model_route_class: ModelRouteClass::ApprovedHostedApi,
            model_base_url: "https://generativelanguage.googleapis.com/v1beta".into(),
            allowed_egress_hosts: vec!["generativelanguage.googleapis.com".into()],
            external_telemetry: false,
        };

        let view = config.view().expect("approved route should be valid");
        assert_eq!(view.model_route, HOSTED_GEMMA_ROUTE);
        assert_eq!(view.qualified_product_claim, GOVERNED_HOSTED_TAGLINE);
        assert_eq!(view.external_telemetry, EXTERNAL_TELEMETRY_DISABLED);
    }

    #[test]
    fn private_route_class_cannot_mislabel_a_public_endpoint() {
        let config = PrivacyBoundaryConfig {
            profile: PrivacyProfile::ApprovedApi,
            model_route_class: ModelRouteClass::CustomerVpcPrivateEndpoint,
            model_base_url: "https://generativelanguage.googleapis.com/v1beta".into(),
            allowed_egress_hosts: vec!["generativelanguage.googleapis.com".into()],
            external_telemetry: false,
        };

        let error = config
            .validate()
            .expect_err("a public endpoint must not be labeled customer-private");
        assert!(error.to_string().contains("private route class"));
    }
}
