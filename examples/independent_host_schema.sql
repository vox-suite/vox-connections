-- Minimal host-owned identity and agent tables for the connector schema example.
-- A production host should integrate these columns and constraints into its own model.
CREATE TABLE public.users (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid()
);
CREATE TABLE public.platform_deployments (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    external_key text NOT NULL UNIQUE
);
CREATE TABLE public.user_contexts (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    deployment_id uuid NOT NULL REFERENCES public.platform_deployments(id),
    user_id uuid NOT NULL REFERENCES public.users(id),
    UNIQUE (id, user_id)
);
CREATE TABLE public.agent_definitions (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    deployment_id uuid NOT NULL REFERENCES public.platform_deployments(id),
    external_key text NOT NULL,
    template_id uuid REFERENCES public.agent_definitions(id),
    owner_user_context_id uuid NOT NULL REFERENCES public.user_contexts(id),
    state text NOT NULL DEFAULT 'enabled',
    instruction_version integer NOT NULL DEFAULT 1,
    requested_capability_categories text[] NOT NULL DEFAULT '{}',
    UNIQUE (owner_user_context_id, external_key)
);
CREATE TABLE public.deployment_agent_selections (
    deployment_id uuid NOT NULL REFERENCES public.platform_deployments(id),
    agent_definition_id uuid NOT NULL REFERENCES public.agent_definitions(id),
    PRIMARY KEY (deployment_id, agent_definition_id)
);
