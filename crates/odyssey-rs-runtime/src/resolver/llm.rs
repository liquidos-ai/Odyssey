use crate::RuntimeError;
use autoagents_llm::{
    HasConfig, LLMProvider as AutoAgentsLLMProvider,
    backends::{
        anthropic::Anthropic, azure_openai::AzureOpenAI, deepseek::DeepSeek, google::Google,
        groq::Groq, minimax::MiniMax, openai::OpenAI, openrouter::OpenRouter, phind::Phind,
        xai::XAI,
    },
    builder::LLMBuilder,
    chat::ReasoningEffort,
};
use odyssey_rs_protocol::ModelSpec;
use serde::Deserialize;
use serde_json::Value;
use std::{env, sync::Arc};

type DynLLMProvider = Arc<dyn AutoAgentsLLMProvider>;

macro_rules! apply_option {
    ($builder:expr, $value:expr, $method:ident) => {{
        if let Some(value) = $value {
            $builder.$method(value)
        } else {
            $builder
        }
    }};
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LLMProvider {
    Cloud(CloudLLMProvider),
    Local(LocalLLMProvider),
    Unknown,
}

impl From<&str> for LLMProvider {
    fn from(value: &str) -> Self {
        let cloud = CloudLLMProvider::from(value);
        if cloud != CloudLLMProvider::Unknown {
            return Self::Cloud(cloud);
        }

        let local = LocalLLMProvider::from(value);
        if local != LocalLLMProvider::Unknown {
            return Self::Local(local);
        }

        Self::Unknown
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloudLLMProvider {
    Anthropic,
    AzureOpenAI,
    DeepSeek,
    Google,
    Groq,
    MiniMax,
    OpenAI,
    OpenRouter,
    Phind,
    #[allow(clippy::upper_case_acronyms)]
    Xai,
    Unknown,
}

impl From<&str> for CloudLLMProvider {
    fn from(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "anthropic" => Self::Anthropic,
            "azure-openai" | "azure_openai" => Self::AzureOpenAI,
            "deepseek" => Self::DeepSeek,
            "google" | "gemini" => Self::Google,
            "groq" => Self::Groq,
            "minimax" | "mini-max" | "mini_max" => Self::MiniMax,
            "openai" => Self::OpenAI,
            "open-router" | "openrouter" => Self::OpenRouter,
            "phind" => Self::Phind,
            "xai" => Self::Xai,
            _ => Self::Unknown,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocalLLMProvider {
    LlamaCpp,
    Unknown,
}

impl From<&str> for LocalLLMProvider {
    fn from(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "llamacpp" | "llama-cpp" | "llama_cpp" => Self::LlamaCpp,
            _ => Self::Unknown,
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
struct LLMConfig {
    #[serde(alias = "apiKey")]
    api_key: Option<String>,
    #[serde(alias = "baseUrl")]
    base_url: Option<String>,
    #[serde(alias = "maxTokens")]
    max_tokens: Option<u32>,
    temperature: Option<f32>,
    #[serde(alias = "timeoutSeconds")]
    timeout_seconds: Option<u64>,
    #[serde(alias = "topP")]
    top_p: Option<f32>,
    #[serde(alias = "topK")]
    top_k: Option<u32>,
    #[serde(alias = "reasoningEffort")]
    reasoning_effort: Option<String>,
    reasoning: Option<bool>,
    #[serde(alias = "reasoningBudgetTokens")]
    reasoning_budget_tokens: Option<u32>,
    #[serde(alias = "enableParallelToolUse")]
    enable_parallel_tool_use: Option<bool>,
    #[serde(alias = "normalizeResponse")]
    normalize_response: Option<bool>,
    #[serde(alias = "embeddingEncodingFormat")]
    embedding_encoding_format: Option<String>,
    #[serde(alias = "embeddingDimensions")]
    embedding_dimensions: Option<u32>,
    #[serde(alias = "apiVersion")]
    api_version: Option<String>,
    #[serde(alias = "deploymentId")]
    deployment_id: Option<String>,
    #[serde(alias = "extraBody")]
    extra_body: Option<Value>,
}

pub(crate) struct LLMResolver<'a> {
    model_spec: &'a ModelSpec,
}

impl<'a> LLMResolver<'a> {
    pub fn new(model_spec: &'a ModelSpec) -> Self {
        Self { model_spec }
    }

    pub fn build_llm(&self) -> Result<DynLLMProvider, RuntimeError> {
        let config = self.parse_config()?;

        match LLMProvider::from(self.model_spec.provider.as_str()) {
            LLMProvider::Cloud(provider) => self.build_cloud_llm(provider, &config),
            LLMProvider::Local(provider) => self.build_local_llm(provider),
            LLMProvider::Unknown => Err(RuntimeError::Unsupported(format!(
                "unsupported model provider: {}",
                self.model_spec.provider
            ))),
        }
    }

    fn build_local_llm(&self, _provider: LocalLLMProvider) -> Result<DynLLMProvider, RuntimeError> {
        Err(RuntimeError::Unsupported(
            "local model support is not yet implemented".to_string(),
        ))
    }

    fn build_cloud_llm(
        &self,
        provider: CloudLLMProvider,
        config: &LLMConfig,
    ) -> Result<DynLLMProvider, RuntimeError> {
        match provider {
            CloudLLMProvider::Anthropic => {
                let api_key = self.require_api_key(config, "anthropic", &["ANTHROPIC_API_KEY"])?;

                let mut builder = LLMBuilder::<Anthropic>::new()
                    .api_key(api_key)
                    .model(self.model_spec.name.as_str());
                builder = self.apply_shared_generation_config(builder, config);
                builder = apply_option!(builder, config.reasoning, reasoning);
                builder = apply_option!(
                    builder,
                    config.reasoning_budget_tokens,
                    reasoning_budget_tokens
                );

                let llm: DynLLMProvider = builder.build().map_err(Self::map_provider_error)?;
                Ok(llm)
            }
            CloudLLMProvider::AzureOpenAI => {
                let api_key =
                    self.require_api_key(config, "azure-openai", &["AZURE_OPENAI_API_KEY"])?;
                let endpoint = self.require_string_setting(
                    config.base_url.as_deref(),
                    "azure-openai",
                    "base_url",
                    &["AZURE_OPENAI_ENDPOINT"],
                )?;
                let api_version = self.require_string_setting(
                    config.api_version.as_deref(),
                    "azure-openai",
                    "api_version",
                    &["AZURE_OPENAI_API_VERSION"],
                )?;
                let deployment_id = self.require_string_setting(
                    config.deployment_id.as_deref(),
                    "azure-openai",
                    "deployment_id",
                    &["AZURE_OPENAI_DEPLOYMENT_ID"],
                )?;

                let mut builder = LLMBuilder::<AzureOpenAI>::new()
                    .api_key(api_key)
                    .base_url(endpoint)
                    .api_version(api_version)
                    .deployment_id(deployment_id)
                    .model(self.model_spec.name.as_str());
                builder = self.apply_shared_generation_config(builder, config);
                builder = self.apply_reasoning_effort(builder, config, "azure-openai")?;
                builder = apply_option!(
                    builder,
                    config.embedding_encoding_format.as_deref(),
                    embedding_encoding_format
                );
                builder = apply_option!(builder, config.embedding_dimensions, embedding_dimensions);

                let llm: DynLLMProvider = builder.build().map_err(Self::map_provider_error)?;
                Ok(llm)
            }
            CloudLLMProvider::DeepSeek => {
                let api_key = self.require_api_key(config, "deepseek", &["DEEPSEEK_API_KEY"])?;

                let llm: DynLLMProvider = Arc::new(DeepSeek::new_with_options(
                    api_key,
                    config.base_url.clone(),
                    Some(self.model_spec.name.clone()),
                    config.max_tokens,
                    config.temperature,
                    config.timeout_seconds,
                    config.top_p,
                    None,
                ));
                Ok(llm)
            }
            CloudLLMProvider::Google => {
                let api_key =
                    self.require_api_key(config, "google", &["GOOGLE_API_KEY", "GEMINI_API_KEY"])?;

                let mut builder = LLMBuilder::<Google>::new()
                    .api_key(api_key)
                    .model(self.model_spec.name.as_str());
                builder = self.apply_shared_generation_config(builder, config);

                let llm: DynLLMProvider = builder.build().map_err(Self::map_provider_error)?;
                Ok(llm)
            }
            CloudLLMProvider::Groq => {
                let api_key = self.require_api_key(config, "groq", &["GROQ_API_KEY"])?;

                let mut builder = LLMBuilder::<Groq>::new()
                    .api_key(api_key)
                    .model(self.model_spec.name.as_str());
                builder = self.apply_shared_generation_config(builder, config);
                builder = self.apply_reasoning_effort(builder, config, "groq")?;
                builder = apply_option!(
                    builder,
                    config.enable_parallel_tool_use,
                    enable_parallel_tool_use
                );
                builder = apply_option!(builder, config.normalize_response, normalize_response);
                builder = apply_option!(builder, config.extra_body.as_ref(), extra_body);

                let llm: DynLLMProvider = builder.build().map_err(Self::map_provider_error)?;
                Ok(llm)
            }
            CloudLLMProvider::MiniMax => {
                let api_key = self.require_api_key(config, "minimax", &["MINIMAX_API_KEY"])?;

                let mut builder = LLMBuilder::<MiniMax>::new()
                    .api_key(api_key)
                    .model(self.model_spec.name.as_str());
                builder = self.apply_shared_generation_config(builder, config);
                builder = self.apply_reasoning_effort(builder, config, "minimax")?;
                builder = apply_option!(
                    builder,
                    config.enable_parallel_tool_use,
                    enable_parallel_tool_use
                );
                builder = apply_option!(builder, config.normalize_response, normalize_response);
                builder = apply_option!(builder, config.extra_body.as_ref(), extra_body);

                let llm: DynLLMProvider = builder.build().map_err(Self::map_provider_error)?;
                Ok(llm)
            }
            CloudLLMProvider::OpenAI => {
                let api_key = self.require_api_key(config, "openai", &["OPENAI_API_KEY"])?;

                let mut builder = LLMBuilder::<OpenAI>::new()
                    .api_key(api_key)
                    .model(self.model_spec.name.as_str());
                builder = self.apply_shared_generation_config(builder, config);
                builder = self.apply_reasoning_effort(builder, config, "openai")?;
                builder = apply_option!(
                    builder,
                    config.embedding_encoding_format.as_deref(),
                    embedding_encoding_format
                );
                builder = apply_option!(builder, config.embedding_dimensions, embedding_dimensions);
                builder = apply_option!(builder, config.normalize_response, normalize_response);
                builder = apply_option!(builder, config.extra_body.as_ref(), extra_body);

                let llm: DynLLMProvider = builder.build().map_err(Self::map_provider_error)?;
                Ok(llm)
            }
            CloudLLMProvider::OpenRouter => {
                let api_key =
                    self.require_api_key(config, "openrouter", &["OPENROUTER_API_KEY"])?;

                let mut builder = LLMBuilder::<OpenRouter>::new()
                    .api_key(api_key)
                    .model(self.model_spec.name.as_str());
                builder = self.apply_shared_generation_config(builder, config);
                builder = self.apply_reasoning_effort(builder, config, "openrouter")?;
                builder = apply_option!(
                    builder,
                    config.enable_parallel_tool_use,
                    enable_parallel_tool_use
                );
                builder = apply_option!(builder, config.normalize_response, normalize_response);
                builder = apply_option!(builder, config.extra_body.as_ref(), extra_body);

                let llm: DynLLMProvider = builder.build().map_err(Self::map_provider_error)?;
                Ok(llm)
            }
            CloudLLMProvider::Phind => {
                let mut builder = LLMBuilder::<Phind>::new().model(self.model_spec.name.as_str());
                builder = self.apply_shared_generation_config(builder, config);

                let llm: DynLLMProvider = builder.build().map_err(Self::map_provider_error)?;
                Ok(llm)
            }
            CloudLLMProvider::Xai => {
                let api_key = self.require_api_key(config, "xai", &["XAI_API_KEY"])?;

                let mut builder = LLMBuilder::<XAI>::new()
                    .api_key(api_key)
                    .model(self.model_spec.name.as_str());
                builder = self.apply_shared_generation_config(builder, config);
                builder = apply_option!(
                    builder,
                    config.embedding_encoding_format.as_deref(),
                    embedding_encoding_format
                );
                builder = apply_option!(builder, config.embedding_dimensions, embedding_dimensions);

                let llm: DynLLMProvider = builder.build().map_err(Self::map_provider_error)?;
                Ok(llm)
            }
            CloudLLMProvider::Unknown => Err(RuntimeError::Unsupported(format!(
                "unsupported model provider: {}",
                self.model_spec.provider
            ))),
        }
    }

    fn apply_shared_generation_config<L: AutoAgentsLLMProvider + HasConfig>(
        &self,
        mut builder: LLMBuilder<L>,
        config: &LLMConfig,
    ) -> LLMBuilder<L> {
        builder = apply_option!(builder, config.base_url.as_deref(), base_url);
        builder = apply_option!(builder, config.max_tokens, max_tokens);
        builder = apply_option!(builder, config.temperature, temperature);
        builder = apply_option!(builder, config.timeout_seconds, timeout_seconds);
        builder = apply_option!(builder, config.top_p, top_p);
        builder = apply_option!(builder, config.top_k, top_k);
        builder
    }

    fn apply_reasoning_effort<L: AutoAgentsLLMProvider + HasConfig>(
        &self,
        builder: LLMBuilder<L>,
        config: &LLMConfig,
        provider_id: &str,
    ) -> Result<LLMBuilder<L>, RuntimeError> {
        let Some(reasoning_effort) = config.reasoning_effort.as_deref() else {
            return Ok(builder);
        };

        let reasoning_effort = match reasoning_effort.trim().to_ascii_lowercase().as_str() {
            "low" => ReasoningEffort::Low,
            "medium" => ReasoningEffort::Medium,
            "high" => ReasoningEffort::High,
            _ => {
                return Err(RuntimeError::Unsupported(format!(
                    "provider {provider_id} requires config.reasoning_effort to be one of [low, medium, high]"
                )));
            }
        };

        Ok(builder.reasoning_effort(reasoning_effort))
    }

    fn parse_config(&self) -> Result<LLMConfig, RuntimeError> {
        match self.model_spec.config.as_ref() {
            None | Some(Value::Null) => Ok(LLMConfig::default()),
            Some(value) => serde_json::from_value(value.clone()).map_err(|err| {
                RuntimeError::Unsupported(format!(
                    "invalid model config for provider {}: {err}",
                    self.model_spec.provider
                ))
            }),
        }
    }

    fn require_api_key(
        &self,
        config: &LLMConfig,
        provider_id: &str,
        env_vars: &[&str],
    ) -> Result<String, RuntimeError> {
        self.require_string_setting(config.api_key.as_deref(), provider_id, "api_key", env_vars)
    }

    fn require_string_setting(
        &self,
        config_value: Option<&str>,
        provider_id: &str,
        config_field: &str,
        env_vars: &[&str],
    ) -> Result<String, RuntimeError> {
        if let Some(config_value) = config_value {
            let trimmed = config_value.trim();
            if trimmed.is_empty() {
                return Err(RuntimeError::Unsupported(format!(
                    "provider {provider_id} requires non-empty config.{config_field}"
                )));
            }

            return Ok(trimmed.to_string());
        }

        env_vars
            .iter()
            .find_map(|env_var| {
                env::var(env_var)
                    .ok()
                    .map(|value| value.trim().to_string())
                    .filter(|value| !value.is_empty())
            })
            .ok_or_else(|| {
                RuntimeError::Unsupported(format!(
                    "provider {provider_id} requires config.{config_field} or one of [{}]",
                    env_vars.join(", ")
                ))
            })
    }

    fn map_provider_error(err: autoagents_llm::error::LLMError) -> RuntimeError {
        RuntimeError::Executor(err.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use serde_json::json;

    fn model_spec(provider: &str, config: Option<Value>) -> ModelSpec {
        ModelSpec {
            provider: provider.to_string(),
            name: "test-model".to_string(),
            config,
        }
    }

    #[test]
    fn parses_cloud_provider_aliases() {
        assert_eq!(
            CloudLLMProvider::from("azure-openai"),
            CloudLLMProvider::AzureOpenAI
        );
        assert_eq!(
            CloudLLMProvider::from("azure_openai"),
            CloudLLMProvider::AzureOpenAI
        );
        assert_eq!(
            CloudLLMProvider::from("open-router"),
            CloudLLMProvider::OpenRouter
        );
        assert_eq!(
            CloudLLMProvider::from("openrouter"),
            CloudLLMProvider::OpenRouter
        );
        assert_eq!(CloudLLMProvider::from("gemini"), CloudLLMProvider::Google);
        assert_eq!(CloudLLMProvider::from("minimax"), CloudLLMProvider::MiniMax);
        assert_eq!(CloudLLMProvider::from("phind"), CloudLLMProvider::Phind);
    }

    #[test]
    fn parses_config_with_snake_and_camel_case_fields() {
        let spec = model_spec(
            "openai",
            Some(json!({
                "apiKey": "key",
                "baseUrl": "https://example.com/v1/",
                "maxTokens": 512,
                "temperature": 0.2,
                "timeoutSeconds": 30,
                "topP": 0.9,
                "topK": 32,
                "reasoningEffort": "medium",
                "normalizeResponse": false,
                "embeddingEncodingFormat": "float",
                "embeddingDimensions": 1536,
                "extraBody": { "service_tier": "flex" }
            })),
        );
        let resolver = LLMResolver::new(&spec);

        let config = resolver.parse_config().unwrap();

        assert_eq!(config.api_key, Some("key".to_string()));
        assert_eq!(config.base_url, Some("https://example.com/v1/".to_string()));
        assert_eq!(config.max_tokens, Some(512));
        assert_eq!(config.temperature, Some(0.2));
        assert_eq!(config.timeout_seconds, Some(30));
        assert_eq!(config.top_p, Some(0.9));
        assert_eq!(config.top_k, Some(32));
        assert_eq!(config.reasoning_effort, Some("medium".to_string()));
        assert_eq!(config.normalize_response, Some(false));
        assert_eq!(config.embedding_encoding_format, Some("float".to_string()));
        assert_eq!(config.embedding_dimensions, Some(1536));
        assert_eq!(config.extra_body, Some(json!({ "service_tier": "flex" })));
    }

    #[test]
    fn rejects_unknown_config_fields() {
        let spec = model_spec(
            "openai",
            Some(json!({ "temperature": 0.2, "unknownField": true })),
        );
        let resolver = LLMResolver::new(&spec);

        let err = resolver.parse_config().unwrap_err();
        assert!(err.to_string().contains("invalid model config"));
        assert!(err.to_string().contains("unknownField"));
    }

    #[test]
    fn requires_credentials_for_openai() {
        let spec = model_spec("openai", Some(json!({ "api_key": "   " })));
        let resolver = LLMResolver::new(&spec);

        let err = match resolver.build_llm() {
            Ok(_) => panic!("expected openai config resolution to fail for blank api_key"),
            Err(err) => err,
        };
        assert!(
            err.to_string()
                .contains("provider openai requires non-empty config.api_key")
        );
    }

    #[test]
    fn requires_azure_endpoint_version_and_deployment() {
        let spec = model_spec("azure-openai", Some(json!({ "api_key": "key" })));
        let resolver = LLMResolver::new(&spec);

        let err = match resolver.build_llm() {
            Ok(_) => panic!("expected azure-openai config resolution to fail"),
            Err(err) => err,
        };
        assert!(
            err.to_string()
                .contains("provider azure-openai requires config.base_url")
        );
    }

    #[test]
    fn builds_all_supported_cloud_providers_from_config() {
        let specs = [
            model_spec(
                "openai",
                Some(json!({
                    "api_key": "key",
                    "base_url": "https://example.com/v1/",
                    "max_tokens": 128,
                    "temperature": 0.2,
                    "top_p": 0.9,
                    "normalize_response": false,
                    "extra_body": { "service_tier": "flex" }
                })),
            ),
            model_spec(
                "anthropic",
                Some(json!({
                    "api_key": "key",
                    "max_tokens": 128,
                    "temperature": 0.2,
                    "top_p": 0.9,
                    "top_k": 16,
                    "reasoning": true,
                    "reasoning_budget_tokens": 512
                })),
            ),
            model_spec(
                "azure-openai",
                Some(json!({
                    "api_key": "key",
                    "base_url": "https://example.openai.azure.com/",
                    "api_version": "2024-10-21",
                    "deployment_id": "test-deployment",
                    "max_tokens": 128,
                    "temperature": 0.2,
                    "top_p": 0.9
                })),
            ),
            model_spec(
                "deepseek",
                Some(json!({
                    "api_key": "key",
                    "base_url": "https://example.com/v1/",
                    "max_tokens": 128,
                    "temperature": 0.2,
                    "top_p": 0.9
                })),
            ),
            model_spec(
                "google",
                Some(json!({
                    "api_key": "key",
                    "maxTokens": 128,
                    "temperature": 0.2,
                    "topP": 0.9,
                    "topK": 16
                })),
            ),
            model_spec(
                "groq",
                Some(json!({
                    "api_key": "key",
                    "base_url": "https://example.com/openai/v1/",
                    "max_tokens": 128,
                    "temperature": 0.2,
                    "top_p": 0.9,
                    "top_k": 16,
                    "enable_parallel_tool_use": true,
                    "normalize_response": false,
                    "extra_body": { "service_tier": "flex" }
                })),
            ),
            model_spec(
                "minimax",
                Some(json!({
                    "api_key": "key",
                    "base_url": "https://api.minimaxi.chat/v1/",
                    "max_tokens": 128,
                    "temperature": 0.2,
                    "top_p": 0.9,
                    "top_k": 16,
                    "normalize_response": false
                })),
            ),
            model_spec(
                "openrouter",
                Some(json!({
                    "api_key": "key",
                    "base_url": "https://openrouter.ai/api/v1/",
                    "max_tokens": 128,
                    "temperature": 0.2,
                    "top_p": 0.9,
                    "top_k": 16
                })),
            ),
            model_spec(
                "phind",
                Some(json!({
                    "base_url": "https://example.com/agent/",
                    "max_tokens": 128,
                    "temperature": 0.2,
                    "top_p": 0.9,
                    "top_k": 16
                })),
            ),
            model_spec(
                "xai",
                Some(json!({
                    "api_key": "key",
                    "max_tokens": 128,
                    "temperature": 0.2,
                    "top_p": 0.9,
                    "top_k": 16
                })),
            ),
        ];

        for spec in specs {
            let provider = spec.provider.clone();
            let llm = LLMResolver::new(&spec).build_llm();
            assert!(
                llm.is_ok(),
                "expected provider {provider} to build successfully"
            );
        }
    }
}
