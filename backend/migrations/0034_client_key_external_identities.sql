-- 外部登录身份与自动开通的 Client Key 一一绑定；删除 Key 时解除绑定，下次登录重新开通。
create table client_key_external_identities (
    provider text not null check (provider in ('feishu')),
    tenant_key text not null check (length(tenant_key) between 1 and 128),
    subject text not null check (length(subject) between 1 and 128),
    client_api_key_id text not null unique references client_api_keys(id) on delete cascade,
    display_name text,
    created_at timestamptz not null,
    primary key (provider, tenant_key, subject)
);
