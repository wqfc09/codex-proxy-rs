-- CPR 用户、认证与订阅计费控制面；保留 0001-0016 原迁移字节不变。
-- 本迁移只新增用户系统事实源，不重复上游运行设置、模型定价或图像计费结构。

-- 用户自助操作沿用统一审计表，仅增加明确的主体类型，不冒充管理员。
alter table admin_audit_events drop constraint admin_audit_events_actor_kind_ck;
alter table admin_audit_events add constraint admin_audit_events_actor_kind_ck
  check (actor_kind in ('admin_session', 'admin_api_key', 'system', 'anonymous', 'user_session'));

create table users (
  id text primary key,
  username text not null unique,
  password_hash text not null,
  role text not null,
  enabled boolean not null default true,
  session_version bigint not null default 1,
  max_concurrency_override bigint default 0,
  requests_per_minute_override bigint default 0,
  -- owned Key 动态继承账户身份；空对象表示跟随系统，不在每把 Key 复制配置。
  provider_request_profiles_json jsonb not null default '{}'::jsonb,
  created_at timestamptz not null,
  updated_at timestamptz not null,
  constraint users_id_ck check (length(id) between 1 and 128),
  constraint users_username_ck check (length(username) between 1 and 128),
  constraint users_role_ck check (role in ('admin', 'user')),
  constraint users_session_version_ck check (session_version > 0),
  constraint users_billing_overrides_ck check ((max_concurrency_override is null or max_concurrency_override >= 0) and (requests_per_minute_override is null or requests_per_minute_override >= 0)),
  constraint users_request_profiles_ck check (
    jsonb_typeof(provider_request_profiles_json) = 'object'
    and provider_request_profiles_json - array['openai', 'xai']::text[] = '{}'::jsonb
    and coalesce(jsonb_typeof(provider_request_profiles_json -> 'openai'), 'object') = 'object'
    and coalesce(jsonb_typeof(provider_request_profiles_json -> 'xai'), 'object') = 'object'
  ),
  constraint users_time_ck check (created_at <= updated_at)
);

insert into users (id, username, password_hash, role, enabled, session_version, max_concurrency_override, requests_per_minute_override, created_at, updated_at)
select id, id, password_hash, 'admin', true, 1, null, null, created_at, updated_at
from admin_users
on conflict (id) do nothing;
create index users_created_idx on users (created_at desc, id desc);

create table auth_settings (
  id bigint primary key,
  turnstile_enabled boolean not null default false,
  turnstile_site_key text,
  turnstile_secret_key text,
  admin_session_ttl_minutes bigint,
  user_session_ttl_minutes bigint,
  updated_at timestamptz not null,
  constraint auth_settings_singleton_ck check (id = 1),
  constraint auth_settings_session_ttl_pair_ck check (
    (admin_session_ttl_minutes is null and user_session_ttl_minutes is null)
    or (admin_session_ttl_minutes between 1 and 527040 and user_session_ttl_minutes between 1 and 527040)
  ),
  constraint auth_settings_turnstile_pair_ck check (
    not turnstile_enabled
    or (nullif(btrim(turnstile_site_key), '') is not null and nullif(btrim(turnstile_secret_key), '') is not null)
  )
);
insert into auth_settings (id, turnstile_enabled, updated_at) values (1, false, now());

alter table client_api_keys add column owner_user_id text references users(id) on delete cascade;
create index client_api_keys_owner_created_idx on client_api_keys (owner_user_id, created_at desc, id desc) where owner_user_id is not null;
alter table model_requests add column user_id text;
create index model_requests_user_started_idx on model_requests (user_id, started_at desc, id desc) where user_id is not null;

create table subscription_plans (
  id text primary key,
  name text not null,
  description text,
  enabled boolean not null default true,
  daily_limit_usd numeric(20, 10),
  weekly_limit_usd numeric(20, 10),
  monthly_limit_usd numeric(20, 10),
  is_base boolean not null default false,
  created_at timestamptz not null,
  updated_at timestamptz not null,
  constraint subscription_plans_id_ck check (id ~ '^plan_[0-9a-f]{32}$'),
  constraint subscription_plans_name_ck check (char_length(btrim(name)) between 1 and 100 and name = btrim(name) and name !~ '[[:cntrl:]]'),
  constraint subscription_plans_description_ck check (description is null or (octet_length(description) <= 4096 and description !~ '[[:cntrl:]]')),
  constraint subscription_plans_limits_ck check (
    (daily_limit_usd is null or daily_limit_usd >= 0)
    and (weekly_limit_usd is null or weekly_limit_usd >= 0)
    and (monthly_limit_usd is null or monthly_limit_usd >= 0)
  ),
  constraint subscription_plans_time_ck check (created_at <= updated_at)
);
create unique index subscription_plans_name_uq on subscription_plans (lower(name));
create unique index subscription_plans_one_base_uq on subscription_plans (is_base) where is_base;
do $$
declare existing_base text;
declare base_id text := 'plan_00000000000000000000000000000001';
declare base_name text := '基础套餐';
begin
  select id into existing_base from subscription_plans where is_base for update limit 1;
  if existing_base is null then
    if exists (select 1 from subscription_plans where id = base_id) then
      base_id := 'plan_' || md5(clock_timestamp()::text || random()::text);
    end if;
    if exists (select 1 from subscription_plans where lower(name) = lower(base_name)) then
      base_name := base_name || '-' || substr(base_id, 6, 8);
    end if;
    insert into subscription_plans (id, name, description, enabled, daily_limit_usd, weekly_limit_usd, monthly_limit_usd, is_base, created_at, updated_at)
    values (base_id, base_name, '内置基础套餐', true, 0, 0, 0, true, now(), now());
  else
    update subscription_plans set enabled = true where id = existing_base;
  end if;
end $$;

create table user_subscriptions (
  id text primary key,
  user_id text references users(id) on delete set null,
  plan_id text not null references subscription_plans(id),
  status text not null,
  starts_at timestamptz not null,
  expires_at timestamptz not null,
  revoked_at timestamptz,
  downstream_rate_multiplier numeric(20, 10) not null default 1,
  created_at timestamptz not null,
  updated_at timestamptz not null,
  constraint user_subscriptions_id_ck check (id ~ '^sub_[0-9a-f]{32}$'),
  constraint user_subscriptions_status_ck check (status in ('active', 'revoked')),
  constraint user_subscriptions_multiplier_ck check (downstream_rate_multiplier >= 0),
  constraint user_subscriptions_time_ck check (starts_at < expires_at and created_at <= updated_at and ((status = 'active' and revoked_at is null) or (status = 'revoked' and revoked_at is not null)))
);
create unique index user_subscriptions_one_active_uq on user_subscriptions (user_id) where status = 'active';
create index user_subscriptions_user_history_idx on user_subscriptions (user_id, created_at desc, id desc);
create index user_subscriptions_plan_idx on user_subscriptions (plan_id, status, expires_at);

create table user_account_groups (
  user_id text not null references users(id) on delete cascade,
  account_group_id text not null references account_groups(id) on delete cascade,
  assigned_at timestamptz not null default now(),
  primary key (user_id, account_group_id)
);
create index user_account_groups_group_idx on user_account_groups (account_group_id, user_id);

create table user_budget_windows (
  user_id text primary key references users(id) on delete cascade,
  daily_start timestamptz not null, daily_end timestamptz not null, daily_used_usd numeric(20,10) not null default 0,
  weekly_start timestamptz not null, weekly_end timestamptz not null, weekly_used_usd numeric(20,10) not null default 0,
  monthly_start timestamptz not null, monthly_end timestamptz not null, monthly_used_usd numeric(20,10) not null default 0,
  constraint user_budget_windows_time_ck check (daily_start < daily_end and weekly_start < weekly_end and monthly_start < monthly_end),
  constraint user_budget_windows_amount_ck check (daily_used_usd >= 0 and weekly_used_usd >= 0 and monthly_used_usd >= 0)
);
create table user_budget_credits (
  id text primary key, user_id text not null references users(id) on delete cascade,
  plan_id text not null references subscription_plans(id), subscription_id text references user_subscriptions(id),
  window_kind text not null, window_start timestamptz not null, window_end timestamptz not null,
  amount_usd numeric(20,10) not null, idempotency_key text not null, reason text not null, created_at timestamptz not null default now(),
  constraint user_budget_credits_window_ck check (window_kind in ('daily','weekly','monthly') and window_start < window_end),
  constraint user_budget_credits_amount_ck check (amount_usd >= 0),
  constraint user_budget_credits_reason_ck check (char_length(btrim(reason)) between 1 and 1024)
);
create unique index user_budget_credits_idempotency_uq on user_budget_credits (user_id, idempotency_key);
create index user_budget_credits_window_idx on user_budget_credits (user_id, window_kind, window_start, window_end, created_at desc);

create table user_charge_events (
  request_id text primary key, user_id text references users(id) on delete set null, subscription_id text,
  user_ref text not null, subscription_ref text,
  plan_id text references subscription_plans(id), billing_group_ref text,
  base_amount_usd numeric(20,10) not null, downstream_rate_multiplier numeric(20,10) not null, billed_amount_usd numeric(20,10) not null,
  completed_at timestamptz not null, created_at timestamptz not null default now(),
  constraint user_charge_events_amount_ck check (base_amount_usd >= 0 and downstream_rate_multiplier >= 0 and billed_amount_usd >= 0),
  constraint user_charge_events_user_ref_ck check (length(btrim(user_ref)) > 0),
  constraint user_charge_events_subscription_ref_ck check (subscription_ref is null or length(btrim(subscription_ref)) > 0),
  constraint user_charge_events_subscription_id_fkey foreign key (subscription_id) references user_subscriptions(id) on delete set null
);
create index user_charge_events_user_completed_idx on user_charge_events (user_id, completed_at desc, request_id desc);

-- live FK 删除时可以解除当前关系；ref 列是结算完成时的不可变历史归属。
create function set_user_charge_event_refs() returns trigger
language plpgsql as $$
begin
  if tg_op = 'UPDATE' then
    if old.user_ref is distinct from new.user_ref or old.subscription_ref is distinct from new.subscription_ref then
      raise exception 'user charge event historical refs are immutable';
    end if;
  else
    new.user_ref := coalesce(new.user_ref, new.user_id);
    new.subscription_ref := coalesce(new.subscription_ref, new.subscription_id);
  end if;
  return new;
end;
$$;
create trigger user_charge_events_refs_trg
before insert or update on user_charge_events
for each row execute function set_user_charge_event_refs();

alter table model_requests add column plan_id text references subscription_plans(id), add column subscription_id text references user_subscriptions(id) on delete set null, add column billing_group_ref text, add column downstream_rate_multiplier numeric(20,10), add column downstream_billed_amount numeric(20,10),
  add constraint model_requests_downstream_billing_ck check ((plan_id is null and subscription_id is null and billing_group_ref is null and downstream_rate_multiplier is null) or (user_id is not null and plan_id is not null and downstream_rate_multiplier is not null and downstream_rate_multiplier >= 0)),
  add constraint model_requests_downstream_billed_amount_ck check (downstream_billed_amount is null or downstream_billed_amount >= 0);
create index model_requests_subscription_started_idx on model_requests (subscription_id, started_at desc, id desc) where subscription_id is not null;
alter table model_requests add column username_snapshot text, add column client_api_key_name_snapshot text;
update model_requests mr set username_snapshot = case when mr.user_id is null then null else coalesce((select u.username from users u where u.id = mr.user_id), mr.user_id) end,
  client_api_key_name_snapshot = coalesce((select k.name from client_api_keys k where k.id = mr.client_api_key_ref), mr.client_api_key_ref);
