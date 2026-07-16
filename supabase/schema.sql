-- CodeChat shared history and owner-authorized message actions.
--
-- How to run this:
--   1. Open your project at https://supabase.com/dashboard
--   2. Left sidebar → "SQL Editor" → "New query"
--   3. Paste this whole file → click "Run"
--
-- What it does: creates one tiny table holding the most recent chat messages
-- so people who just joined see the ongoing conversation instead of an empty
-- room. Live delivery still happens over Realtime Broadcast — this table is
-- only read once per client, at join time (last 50 rows).
--
-- Size stays bounded forever: a trigger trims everything older than the
-- newest 1000 rows on each insert, so the free tier's 500 MB is never dented.

create extension if not exists pgcrypto;

create table if not exists public.messages (
  id         bigint generated always as identity primary key,
  username   text        not null check (char_length(username) between 2 and 20),
  text       text        not null check (char_length(text)     between 1 and 300),
  created_at timestamptz not null default now()
);

-- These additions make this file safe to run over an existing CodeChat table.
-- client_id is public and lets a client recognize its own history. The owner
-- hash is never selectable and authorizes edit/delete without requiring login.
alter table public.messages add column if not exists client_id uuid default gen_random_uuid();
alter table public.messages add column if not exists owner_token_hash text;
alter table public.messages add column if not exists edited_at timestamptz;

-- Row Level Security: the publishable/anon key may INSERT and SELECT public
-- columns. UPDATE/DELETE stay unavailable directly; the guarded RPC functions
-- below only change rows when the caller supplies their original owner token.
alter table public.messages enable row level security;

drop policy if exists "anyone may read recent messages" on public.messages;
create policy "anyone may read recent messages"
  on public.messages for select
  to anon
  using (true);

drop policy if exists "anyone may post a message" on public.messages;
create policy "anyone may post a message"
  on public.messages for insert
  to anon
  with check (true);

revoke all on public.messages from anon;
grant select (id, username, text, client_id, created_at, edited_at) on public.messages to anon;
-- Compatibility for older clients. New clients use create_message() below.
grant insert (username, text) on public.messages to anon;

create or replace function public.create_message(
  p_username text,
  p_text text,
  p_client_id uuid,
  p_owner_token text
)
returns table(message_id bigint, message_created_at timestamptz)
language plpgsql
security definer
set search_path = public, extensions
as $$
begin
  if char_length(p_owner_token) < 32 then
    raise exception 'invalid owner token';
  end if;
  return query
    insert into public.messages (username, text, client_id, owner_token_hash)
    values (p_username, p_text, p_client_id, encode(digest(p_owner_token, 'sha256'), 'hex'))
    returning id, created_at;
end;
$$;

create or replace function public.edit_message(
  p_message_id bigint,
  p_text text,
  p_owner_token text
)
returns boolean
language plpgsql
security definer
set search_path = public, extensions
as $$
declare changed integer;
begin
  update public.messages
     set text = p_text, edited_at = now()
   where id = p_message_id
     and owner_token_hash = encode(digest(p_owner_token, 'sha256'), 'hex');
  get diagnostics changed = row_count;
  return changed = 1;
end;
$$;

create or replace function public.delete_message(
  p_message_id bigint,
  p_owner_token text
)
returns boolean
language plpgsql
security definer
set search_path = public, extensions
as $$
declare changed integer;
begin
  delete from public.messages
   where id = p_message_id
     and owner_token_hash = encode(digest(p_owner_token, 'sha256'), 'hex');
  get diagnostics changed = row_count;
  return changed = 1;
end;
$$;

revoke all on function public.create_message(text, text, uuid, text) from public;
revoke all on function public.edit_message(bigint, text, text) from public;
revoke all on function public.delete_message(bigint, text) from public;
grant execute on function public.create_message(text, text, uuid, text) to anon;
grant execute on function public.edit_message(bigint, text, text) to anon;
grant execute on function public.delete_message(bigint, text) to anon;

-- Self-trimming: keep only the newest ~1000 rows. The delete scans by primary
-- key so it's effectively free at chat volumes. SECURITY DEFINER lets the
-- trigger delete even though clients themselves have no delete rights.
create or replace function public.trim_messages()
returns trigger
language plpgsql
security definer
as $$
begin
  delete from public.messages where id < new.id - 1000;
  return null;
end;
$$;

drop trigger if exists trim_messages on public.messages;
create trigger trim_messages
  after insert on public.messages
  for each row
  execute function public.trim_messages();
