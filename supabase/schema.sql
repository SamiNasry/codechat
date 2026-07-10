-- CodeChat shared history (optional feature, run once by the OPERATOR).
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

create table public.messages (
  id         bigint generated always as identity primary key,
  username   text        not null check (char_length(username) between 2 and 20),
  text       text        not null check (char_length(text)     between 1 and 300),
  created_at timestamptz not null default now()
);

-- Row Level Security: the publishable/anon key may INSERT and SELECT, nothing
-- else. No UPDATE/DELETE policies exist, so messages can't be edited or
-- wiped by clients.
alter table public.messages enable row level security;

create policy "anyone may read recent messages"
  on public.messages for select
  to anon
  using (true);

create policy "anyone may post a message"
  on public.messages for insert
  to anon
  with check (true);

grant select, insert on public.messages to anon;

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

create trigger trim_messages
  after insert on public.messages
  for each row
  execute function public.trim_messages();
