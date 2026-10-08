-- Reference connector schema for a private, service-only PostgreSQL database.
-- Hosts create users, user_contexts, platform_deployments, agent_definitions,
-- and deployment_agent_selections before applying this file.
-- Hosts must enforce their own database access policy; do not expose this role to end users.
--
-- PostgreSQL database dump
--


-- Dumped from database version 18.6 (Homebrew)
-- Dumped by pg_dump version 18.6 (Homebrew)

SET statement_timeout = 0;
SET lock_timeout = 0;
SET idle_in_transaction_session_timeout = 0;
SET client_encoding = 'UTF8';
SET standard_conforming_strings = on;
SELECT pg_catalog.set_config('search_path', '', false);
SET check_function_bodies = false;
SET xmloption = content;
SET client_min_messages = warning;
SET row_security = off;

SET default_tablespace = '';

SET default_table_access_method = heap;

--
-- Name: agent_capability_grants; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.agent_capability_grants (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    user_context_id uuid NOT NULL,
    agent_definition_id uuid NOT NULL,
    connection_id uuid NOT NULL,
    capability_external_key text NOT NULL,
    state text DEFAULT 'enabled'::text NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    revoked_at timestamp with time zone,
    CONSTRAINT agent_capability_grants_capability_not_empty CHECK (((length(btrim(capability_external_key)) >= 1) AND (length(btrim(capability_external_key)) <= 511))),
    CONSTRAINT agent_capability_grants_revocation_shape CHECK ((((state = 'revoked'::text) AND (revoked_at IS NOT NULL)) OR ((state = 'enabled'::text) AND (revoked_at IS NULL)))),
    CONSTRAINT agent_capability_grants_state_valid CHECK ((state = ANY (ARRAY['enabled'::text, 'revoked'::text])))
);


--
-- Name: external_connections; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.external_connections (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    user_context_id uuid NOT NULL,
    integration_id uuid,
    remote_extension_id uuid,
    external_account_hash bytea NOT NULL,
    credential_custody text NOT NULL,
    authorization_state text NOT NULL,
    authorized_capabilities text[] DEFAULT '{}'::text[] NOT NULL,
    expires_at timestamp with time zone,
    failure_code text,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    revoked_at timestamp with time zone,
    account_display_id text,
    CONSTRAINT external_connections_account_hash_length CHECK ((octet_length(external_account_hash) = 32)),
    CONSTRAINT external_connections_one_source CHECK (((integration_id IS NOT NULL) <> (remote_extension_id IS NOT NULL))),
    CONSTRAINT external_connections_custody_valid CHECK ((credential_custody = ANY (ARRAY['platform_held'::text, 'external_operator'::text]))),
    CONSTRAINT external_connections_expiry_shape CHECK (((authorization_state = 'authorized'::text) OR (expires_at IS NULL))),
    CONSTRAINT external_connections_failure_shape CHECK ((((authorization_state = 'failed'::text) AND (failure_code IS NOT NULL)) OR (authorization_state <> 'failed'::text))),
    CONSTRAINT external_connections_state_valid CHECK ((authorization_state = ANY (ARRAY['pending'::text, 'authorized'::text, 'expired'::text, 'revoked'::text, 'cancelled'::text, 'failed'::text])))
);


--
-- Name: integration_capability_declarations; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.integration_capability_declarations (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    integration_id uuid NOT NULL,
    external_key text NOT NULL,
    effect text NOT NULL,
    access_needs text[] DEFAULT '{}'::text[] NOT NULL,
    data_recipients text[] DEFAULT '{}'::text[] NOT NULL,
    regions text[] DEFAULT '{}'::text[] NOT NULL,
    failure_modes text[] DEFAULT '{}'::text[] NOT NULL,
    optional_guarantees jsonb DEFAULT '{}'::jsonb CONSTRAINT integration_capability_declaration_optional_guarantees_not_null NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    CONSTRAINT integration_capabilities_effect_valid CHECK ((effect = ANY (ARRAY['read'::text, 'write'::text, 'mixed'::text]))),
    CONSTRAINT integration_capabilities_guarantees_object CHECK ((jsonb_typeof(optional_guarantees) = 'object'::text)),
    CONSTRAINT integration_capabilities_key_not_empty CHECK (((length(btrim(external_key)) >= 1) AND (length(btrim(external_key)) <= 255)))
);


--
-- Name: integration_declaration_versions; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.integration_declaration_versions (
    integration_id uuid NOT NULL,
    version integer NOT NULL,
    declaration jsonb NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    CONSTRAINT integration_declaration_versions_declaration_check CHECK ((jsonb_typeof(declaration) = 'object'::text)),
    CONSTRAINT integration_declaration_versions_version_check CHECK ((version > 0))
);


--
-- Name: integration_definitions; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.integration_definitions (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    deployment_id uuid NOT NULL,
    external_key text NOT NULL,
    protocol text NOT NULL,
    display_name text NOT NULL,
    declaration_version integer NOT NULL,
    state text DEFAULT 'disabled'::text NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    CONSTRAINT integration_definitions_key_not_empty CHECK (((length(btrim(external_key)) >= 1) AND (length(btrim(external_key)) <= 255))),
    CONSTRAINT integration_definitions_name_not_empty CHECK (((length(btrim(display_name)) >= 1) AND (length(btrim(display_name)) <= 255))),
    CONSTRAINT integration_definitions_protocol_valid CHECK ((protocol = ANY (ARRAY['mcp'::text, 'direct'::text]))),
    CONSTRAINT integration_definitions_state_valid CHECK ((state = ANY (ARRAY['enabled'::text, 'disabled'::text]))),
    CONSTRAINT integration_definitions_version_valid CHECK ((declaration_version > 0))
);


--
-- Name: mcp_oauth_clients; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.mcp_oauth_clients (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    issuer text NOT NULL,
    redirect_uri text NOT NULL,
    client_id text NOT NULL,
    client_secret_ciphertext bytea,
    token_endpoint_auth_method text DEFAULT 'none'::text NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL
);


--
-- Name: mcp_authorization_sessions; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.mcp_authorization_sessions (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    extension_id uuid NOT NULL,
    user_context_id uuid NOT NULL,
    state_hash text NOT NULL,
    code_verifier_ciphertext bytea NOT NULL,
    issuer text NOT NULL,
    token_endpoint text NOT NULL,
    client_id text NOT NULL,
    redirect_uri text NOT NULL,
    resource text NOT NULL,
    endpoint_url text NOT NULL,
    extension_version integer NOT NULL,
    expires_at timestamp with time zone NOT NULL,
    consumed_at timestamp with time zone,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    CONSTRAINT mcp_authorization_sessions_version_positive CHECK ((extension_version > 0))
);


--
-- Name: remote_extension_conformance_runs; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.remote_extension_conformance_runs (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    extension_id uuid NOT NULL,
    version integer NOT NULL,
    status text NOT NULL,
    report jsonb DEFAULT '{}'::jsonb NOT NULL,
    run_at timestamp with time zone DEFAULT now() NOT NULL,
    CONSTRAINT remote_extension_conformance_runs_status_check CHECK ((status = ANY (ARRAY['passed'::text, 'failed'::text])))
);


--
-- Name: remote_extension_credentials; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.remote_extension_credentials (
    extension_id uuid NOT NULL,
    issuer text NOT NULL,
    token_endpoint text NOT NULL,
    client_id text NOT NULL,
    resource text NOT NULL,
    access_token_ciphertext bytea NOT NULL,
    refresh_token_ciphertext bytea,
    scope text,
    expires_at timestamp with time zone,
    server_info jsonb DEFAULT '{}'::jsonb NOT NULL,
    tools jsonb DEFAULT '[]'::jsonb NOT NULL,
    connected_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    tools_refreshed_at timestamp with time zone DEFAULT now() NOT NULL
);


--
-- Name: remote_extension_versions; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.remote_extension_versions (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    extension_id uuid NOT NULL,
    version integer NOT NULL,
    endpoint_url text NOT NULL,
    operator_id text NOT NULL,
    operator_name text NOT NULL,
    capabilities jsonb DEFAULT '[]'::jsonb NOT NULL,
    conformance_status text DEFAULT 'pending'::text NOT NULL,
    conformance_report jsonb DEFAULT '{}'::jsonb NOT NULL,
    consent_granted_at timestamp with time zone,
    quarantined_at timestamp with time zone,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    CONSTRAINT remote_extension_versions_conformance_status_check CHECK ((conformance_status = ANY (ARRAY['pending'::text, 'passed'::text, 'failed'::text]))),
    CONSTRAINT remote_extension_versions_version_check CHECK ((version > 0))
);


--
-- Name: remote_extensions; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.remote_extensions (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    user_context_id uuid NOT NULL,
    external_key text NOT NULL,
    display_name text NOT NULL,
    protocol text NOT NULL,
    endpoint_url text NOT NULL,
    operator_id text NOT NULL,
    operator_name text NOT NULL,
    support_email text,
    terms_url text,
    current_version integer DEFAULT 1 NOT NULL,
    conformance_status text DEFAULT 'pending'::text NOT NULL,
    operator_enabled boolean DEFAULT false NOT NULL,
    consent_status text DEFAULT 'consented'::text NOT NULL,
    lifecycle_state text DEFAULT 'installed'::text NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    CONSTRAINT remote_extensions_conformance_status_check CHECK ((conformance_status = ANY (ARRAY['pending'::text, 'passed'::text, 'failed'::text]))),
    CONSTRAINT remote_extensions_consent_status_check CHECK ((consent_status = ANY (ARRAY['consented'::text, 'consent_required'::text]))),
    CONSTRAINT remote_extensions_current_version_check CHECK ((current_version > 0)),
    CONSTRAINT remote_extensions_lifecycle_state_check CHECK ((lifecycle_state = ANY (ARRAY['installed'::text, 'active'::text, 'quarantined'::text, 'disabled'::text, 'removed'::text]))),
    CONSTRAINT remote_extensions_protocol_check CHECK ((protocol = ANY (ARRAY['mcp'::text, 'direct'::text])))
);


--
-- Name: skill_agent_enablements; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.skill_agent_enablements (
    user_context_id uuid NOT NULL,
    skill_id uuid NOT NULL,
    agent_definition_id uuid NOT NULL,
    enabled boolean DEFAULT true NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL
);


--
-- Name: skill_installations; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.skill_installations (
    user_context_id uuid NOT NULL,
    skill_id uuid NOT NULL,
    installed_version integer NOT NULL,
    enabled boolean DEFAULT true NOT NULL,
    installed_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL
);


--
-- Name: skill_package_versions; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.skill_package_versions (
    skill_id uuid NOT NULL,
    version integer NOT NULL,
    instructions text NOT NULL,
    requested_capabilities text[] DEFAULT '{}'::text[] NOT NULL,
    resources jsonb DEFAULT '{}'::jsonb NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    CONSTRAINT skill_package_versions_version_check CHECK ((version > 0))
);


--
-- Name: skill_packages; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.skill_packages (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    deployment_id uuid NOT NULL,
    owner_user_context_id uuid,
    external_key text NOT NULL,
    title text NOT NULL,
    summary text NOT NULL,
    latest_version integer DEFAULT 1 NOT NULL,
    state text DEFAULT 'active'::text NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    CONSTRAINT skill_packages_latest_version_check CHECK ((latest_version > 0)),
    CONSTRAINT skill_packages_state_check CHECK ((state = ANY (ARRAY['active'::text, 'removed'::text])))
);


--
-- Name: agent_capability_grants agent_capability_grants_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.agent_capability_grants
    ADD CONSTRAINT agent_capability_grants_pkey PRIMARY KEY (id);


--
-- Name: agent_capability_grants agent_capability_grants_unique_scope; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.agent_capability_grants
    ADD CONSTRAINT agent_capability_grants_unique_scope UNIQUE (user_context_id, agent_definition_id, connection_id, capability_external_key);


--
-- Name: external_connections external_connections_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.external_connections
    ADD CONSTRAINT external_connections_pkey PRIMARY KEY (id);


--
-- Name: external_connections external_connections_id_context_key; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.external_connections
    ADD CONSTRAINT external_connections_id_context_key UNIQUE (id, user_context_id);


--
-- Name: external_connections external_connections_unique_account; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.external_connections
    ADD CONSTRAINT external_connections_unique_account UNIQUE (user_context_id, integration_id, external_account_hash);


--
-- Name: integration_capability_declarations integration_capabilities_key_unique; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.integration_capability_declarations
    ADD CONSTRAINT integration_capabilities_key_unique UNIQUE (integration_id, external_key);


--
-- Name: integration_capability_declarations integration_capability_declarations_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.integration_capability_declarations
    ADD CONSTRAINT integration_capability_declarations_pkey PRIMARY KEY (id);


--
-- Name: integration_declaration_versions integration_declaration_versions_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.integration_declaration_versions
    ADD CONSTRAINT integration_declaration_versions_pkey PRIMARY KEY (integration_id, version);


--
-- Name: integration_definitions integration_definitions_deployment_id_unique; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.integration_definitions
    ADD CONSTRAINT integration_definitions_deployment_id_unique UNIQUE (deployment_id, id);


--
-- Name: integration_definitions integration_definitions_key_unique; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.integration_definitions
    ADD CONSTRAINT integration_definitions_key_unique UNIQUE (deployment_id, external_key);


--
-- Name: integration_definitions integration_definitions_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.integration_definitions
    ADD CONSTRAINT integration_definitions_pkey PRIMARY KEY (id);


--
-- Name: mcp_oauth_clients mcp_oauth_clients_issuer_redirect_unique; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.mcp_oauth_clients
    ADD CONSTRAINT mcp_oauth_clients_issuer_redirect_unique UNIQUE (issuer, redirect_uri);


--
-- Name: mcp_oauth_clients mcp_oauth_clients_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.mcp_oauth_clients
    ADD CONSTRAINT mcp_oauth_clients_pkey PRIMARY KEY (id);


--
-- Name: mcp_authorization_sessions mcp_authorization_sessions_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.mcp_authorization_sessions
    ADD CONSTRAINT mcp_authorization_sessions_pkey PRIMARY KEY (id);


--
-- Name: mcp_authorization_sessions mcp_authorization_sessions_state_hash_key; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.mcp_authorization_sessions
    ADD CONSTRAINT mcp_authorization_sessions_state_hash_key UNIQUE (state_hash);


--
-- Name: remote_extension_conformance_runs remote_extension_conformance_runs_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.remote_extension_conformance_runs
    ADD CONSTRAINT remote_extension_conformance_runs_pkey PRIMARY KEY (id);


--
-- Name: remote_extension_credentials remote_extension_credentials_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.remote_extension_credentials
    ADD CONSTRAINT remote_extension_credentials_pkey PRIMARY KEY (extension_id);


--
-- Name: remote_extension_versions remote_extension_versions_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.remote_extension_versions
    ADD CONSTRAINT remote_extension_versions_pkey PRIMARY KEY (id);


--
-- Name: remote_extension_versions remote_extension_versions_unique; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.remote_extension_versions
    ADD CONSTRAINT remote_extension_versions_unique UNIQUE (extension_id, version);


--
-- Name: remote_extensions remote_extensions_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.remote_extensions
    ADD CONSTRAINT remote_extensions_pkey PRIMARY KEY (id);


--
-- Name: remote_extensions remote_extensions_id_context_key; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.remote_extensions
    ADD CONSTRAINT remote_extensions_id_context_key UNIQUE (id, user_context_id);


--
-- Name: remote_extensions remote_extensions_user_key_unique; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.remote_extensions
    ADD CONSTRAINT remote_extensions_user_key_unique UNIQUE (user_context_id, external_key);


--
-- Name: skill_agent_enablements skill_agent_enablements_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.skill_agent_enablements
    ADD CONSTRAINT skill_agent_enablements_pkey PRIMARY KEY (user_context_id, skill_id, agent_definition_id);


--
-- Name: skill_installations skill_installations_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.skill_installations
    ADD CONSTRAINT skill_installations_pkey PRIMARY KEY (user_context_id, skill_id);


--
-- Name: skill_package_versions skill_package_versions_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.skill_package_versions
    ADD CONSTRAINT skill_package_versions_pkey PRIMARY KEY (skill_id, version);


--
-- Name: skill_packages skill_packages_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.skill_packages
    ADD CONSTRAINT skill_packages_pkey PRIMARY KEY (id);


--
-- Name: agent_capability_grants_context_agent_idx; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX agent_capability_grants_context_agent_idx ON public.agent_capability_grants USING btree (user_context_id, agent_definition_id) WHERE (state = 'enabled'::text);


--
--
-- Name: external_connections_context_state_idx; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX external_connections_context_state_idx ON public.external_connections USING btree (user_context_id, authorization_state);


CREATE UNIQUE INDEX external_connections_one_account_per_remote_extension ON public.external_connections USING btree (user_context_id, remote_extension_id) WHERE (remote_extension_id IS NOT NULL);


--
-- Name: integration_definitions_enabled_idx; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX integration_definitions_enabled_idx ON public.integration_definitions USING btree (deployment_id, external_key) WHERE (state = 'enabled'::text);


--
-- Name: remote_extensions_user_state_idx; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX remote_extensions_user_state_idx ON public.remote_extensions USING btree (user_context_id, lifecycle_state);


--
-- Name: skill_packages_curated_key; Type: INDEX; Schema: public; Owner: -
--

CREATE UNIQUE INDEX skill_packages_curated_key ON public.skill_packages USING btree (deployment_id, external_key) WHERE (owner_user_context_id IS NULL);


--
-- Name: skill_packages_private_key; Type: INDEX; Schema: public; Owner: -
--

CREATE UNIQUE INDEX skill_packages_private_key ON public.skill_packages USING btree (owner_user_context_id, external_key) WHERE (owner_user_context_id IS NOT NULL);


--
-- Name: agent_capability_grants agent_capability_grants_agent_definition_id_fkey; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.agent_capability_grants
    ADD CONSTRAINT agent_capability_grants_agent_definition_id_fkey FOREIGN KEY (agent_definition_id) REFERENCES public.agent_definitions(id) ON DELETE RESTRICT;


--
-- Name: agent_capability_grants agent_capability_grants_connection_context_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.agent_capability_grants
    ADD CONSTRAINT agent_capability_grants_connection_context_fk FOREIGN KEY (connection_id, user_context_id) REFERENCES public.external_connections(id, user_context_id) ON DELETE RESTRICT;


--
-- Name: agent_capability_grants agent_capability_grants_user_context_id_fkey; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.agent_capability_grants
    ADD CONSTRAINT agent_capability_grants_user_context_id_fkey FOREIGN KEY (user_context_id) REFERENCES public.user_contexts(id) ON DELETE RESTRICT;


--
-- Name: external_connections external_connections_integration_id_fkey; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.external_connections
    ADD CONSTRAINT external_connections_integration_id_fkey FOREIGN KEY (integration_id) REFERENCES public.integration_definitions(id) ON DELETE RESTRICT;


--
-- Name: external_connections external_connections_remote_extension_context_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.external_connections
    ADD CONSTRAINT external_connections_remote_extension_context_fk FOREIGN KEY (remote_extension_id, user_context_id) REFERENCES public.remote_extensions(id, user_context_id) ON DELETE RESTRICT;


--
-- Name: external_connections external_connections_user_context_id_fkey; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.external_connections
    ADD CONSTRAINT external_connections_user_context_id_fkey FOREIGN KEY (user_context_id) REFERENCES public.user_contexts(id) ON DELETE RESTRICT;


--
-- Name: integration_capability_declarations integration_capability_declarations_integration_id_fkey; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.integration_capability_declarations
    ADD CONSTRAINT integration_capability_declarations_integration_id_fkey FOREIGN KEY (integration_id) REFERENCES public.integration_definitions(id) ON DELETE RESTRICT;


--
-- Name: integration_declaration_versions integration_declaration_versions_integration_id_fkey; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.integration_declaration_versions
    ADD CONSTRAINT integration_declaration_versions_integration_id_fkey FOREIGN KEY (integration_id) REFERENCES public.integration_definitions(id) ON DELETE RESTRICT;


--
-- Name: integration_definitions integration_definitions_deployment_id_fkey; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.integration_definitions
    ADD CONSTRAINT integration_definitions_deployment_id_fkey FOREIGN KEY (deployment_id) REFERENCES public.platform_deployments(id) ON DELETE RESTRICT;


--
-- Name: mcp_authorization_sessions mcp_authorization_sessions_extension_id_fkey; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.mcp_authorization_sessions
    ADD CONSTRAINT mcp_authorization_sessions_extension_id_fkey FOREIGN KEY (extension_id) REFERENCES public.remote_extensions(id) ON DELETE CASCADE;


--
-- Name: mcp_authorization_sessions mcp_authorization_sessions_user_context_id_fkey; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.mcp_authorization_sessions
    ADD CONSTRAINT mcp_authorization_sessions_user_context_id_fkey FOREIGN KEY (user_context_id) REFERENCES public.user_contexts(id) ON DELETE CASCADE;


--
-- Name: remote_extension_conformance_runs remote_extension_conformance_runs_extension_id_fkey; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.remote_extension_conformance_runs
    ADD CONSTRAINT remote_extension_conformance_runs_extension_id_fkey FOREIGN KEY (extension_id) REFERENCES public.remote_extensions(id) ON DELETE RESTRICT;


--
-- Name: remote_extension_credentials remote_extension_credentials_extension_id_fkey; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.remote_extension_credentials
    ADD CONSTRAINT remote_extension_credentials_extension_id_fkey FOREIGN KEY (extension_id) REFERENCES public.remote_extensions(id) ON DELETE CASCADE;


--
-- Name: remote_extension_versions remote_extension_versions_extension_id_fkey; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.remote_extension_versions
    ADD CONSTRAINT remote_extension_versions_extension_id_fkey FOREIGN KEY (extension_id) REFERENCES public.remote_extensions(id) ON DELETE RESTRICT;


--
-- Name: remote_extensions remote_extensions_user_context_id_fkey; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.remote_extensions
    ADD CONSTRAINT remote_extensions_user_context_id_fkey FOREIGN KEY (user_context_id) REFERENCES public.user_contexts(id) ON DELETE RESTRICT;


--
-- Name: skill_agent_enablements skill_agent_enablements_agent_definition_id_fkey; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.skill_agent_enablements
    ADD CONSTRAINT skill_agent_enablements_agent_definition_id_fkey FOREIGN KEY (agent_definition_id) REFERENCES public.agent_definitions(id) ON DELETE CASCADE;


--
-- Name: skill_agent_enablements skill_agent_enablements_skill_id_fkey; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.skill_agent_enablements
    ADD CONSTRAINT skill_agent_enablements_skill_id_fkey FOREIGN KEY (skill_id) REFERENCES public.skill_packages(id) ON DELETE RESTRICT;


--
-- Name: skill_agent_enablements skill_agent_enablements_user_context_id_fkey; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.skill_agent_enablements
    ADD CONSTRAINT skill_agent_enablements_user_context_id_fkey FOREIGN KEY (user_context_id) REFERENCES public.user_contexts(id) ON DELETE CASCADE;


--
-- Name: skill_agent_enablements skill_agent_enablements_user_context_id_skill_id_fkey; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.skill_agent_enablements
    ADD CONSTRAINT skill_agent_enablements_user_context_id_skill_id_fkey FOREIGN KEY (user_context_id, skill_id) REFERENCES public.skill_installations(user_context_id, skill_id) ON DELETE CASCADE;


--
-- Name: skill_installations skill_installations_skill_id_fkey; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.skill_installations
    ADD CONSTRAINT skill_installations_skill_id_fkey FOREIGN KEY (skill_id) REFERENCES public.skill_packages(id) ON DELETE RESTRICT;


--
-- Name: skill_installations skill_installations_skill_id_installed_version_fkey; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.skill_installations
    ADD CONSTRAINT skill_installations_skill_id_installed_version_fkey FOREIGN KEY (skill_id, installed_version) REFERENCES public.skill_package_versions(skill_id, version);


--
-- Name: skill_installations skill_installations_user_context_id_fkey; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.skill_installations
    ADD CONSTRAINT skill_installations_user_context_id_fkey FOREIGN KEY (user_context_id) REFERENCES public.user_contexts(id) ON DELETE CASCADE;


--
-- Name: skill_package_versions skill_package_versions_skill_id_fkey; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.skill_package_versions
    ADD CONSTRAINT skill_package_versions_skill_id_fkey FOREIGN KEY (skill_id) REFERENCES public.skill_packages(id) ON DELETE RESTRICT;


--
-- Name: skill_packages skill_packages_deployment_id_fkey; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.skill_packages
    ADD CONSTRAINT skill_packages_deployment_id_fkey FOREIGN KEY (deployment_id) REFERENCES public.platform_deployments(id) ON DELETE CASCADE;


--
-- Name: skill_packages skill_packages_owner_user_context_id_fkey; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.skill_packages
    ADD CONSTRAINT skill_packages_owner_user_context_id_fkey FOREIGN KEY (owner_user_context_id) REFERENCES public.user_contexts(id) ON DELETE CASCADE;
