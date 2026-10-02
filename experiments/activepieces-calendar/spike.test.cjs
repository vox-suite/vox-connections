'use strict';
const test = require('node:test');
const assert = require('node:assert/strict');
const nock = require('nock');
const { CalendarPieceAdapter, digest, VERSION } = require('./adapter.cjs');
nock.disableNetConnect();
const readArgs = { calendar_id: 'primary', start_date: '2026-10-02T00:00:00Z', end_date: '2026-10-03T00:00:00Z' };
const writeArgs = { calendar_id: 'primary', title: 'Reviewed meeting', start_date_time: '2026-10-02T10:00:00Z', end_date_time: '2026-10-02T10:30:00Z', attendees: ['guest@example.test'], send_notifications: 'none' };
function fixture() {
  const connection = { context: 'tenant-a', agent: 'assistant-a', id: 'account-a', revoked: false, write: false };
  const proposals = new Map();
  const outcomes = [];
  const port = {
    async claim(c) {
      assert.equal(c.context, connection.context, 'foreign context');
      assert.equal(c.agent, connection.agent, 'foreign agent');
      assert.equal(c.connection, connection.id, 'wrong account');
      assert.equal(connection.revoked, false, 'revoked');
      if (c.effect === 'write') {
        assert.equal(connection.write, true, 'missing service scope');
        const p = proposals.get(c.proposal);
        assert.ok(p?.approved && !p.claimed && p.digest === c.argumentDigest, 'unapproved, changed or replayed proposal');
        p.claimed = true;
      }
      // Synthetic token is never a real credential, refresh token or client secret.
      return { type: 'OAUTH2', access_token: 'synthetic-fixture-token', token_type: 'Bearer', expiry_date: Date.now() + 3600000 };
    },
    async finish(c, status) { outcomes.push({ capability: c.capability, status }); },
  };
  return { adapter: new CalendarPieceAdapter(port), connection, proposals, outcomes, port };
}
function call(capability, arguments_, extra = {}) {
  return { context: 'tenant-a', agent: 'assistant-a', connection: 'account-a', pieceVersion: VERSION, capability, arguments: arguments_, ...extra };
}
test.afterEach(() => { assert.deepEqual(nock.pendingMocks(), []); nock.cleanAll(); });
test('actual published piece reads one reviewed account and marks incomplete paging', async () => {
  const f = fixture();
  const mock = nock('https://www.googleapis.com', { reqheaders: { authorization: 'Bearer synthetic-fixture-token' } })
    .get('/calendar/v3/calendars/primary/events').query(q => q.singleEvents === 'true' && q.timeMin.startsWith('2026-10-02'))
    .reply(200, { items: [{ id: 'event-1', summary: 'Meeting', start: { dateTime: '2026-10-02T10:00:00Z' }, end: {} }], nextPageToken: 'another-page' });
  const result = await f.adapter.execute(call('list_events', readArgs));
  assert.equal(result.events[0].id, 'event-1'); assert.equal(result.incomplete, true); assert.ok(mock.isDone());
});
test('foreign context, wrong account, revoked grant, changed version and unreviewed inputs dispatch nothing', async () => {
  for (const patch of [{ context: 'tenant-b' }, { connection: 'account-b' }, { agent: 'specialist-b' }, { pieceVersion: '0.13.0' }, { arguments: { ...readArgs, arbitrary_url: 'https://evil.test' } }]) {
    const f = fixture(); await assert.rejects(f.adapter.execute(call('list_events', readArgs, patch)));
  }
  const f = fixture(); f.connection.revoked = true; await assert.rejects(f.adapter.execute(call('list_events', readArgs)));
});
test('actual published piece creates exactly approved event without changing recipients or notification policy', async () => {
  const f = fixture(); f.connection.write = true; f.proposals.set('proposal-1', { approved: true, digest: digest(writeArgs) });
  const mock = nock('https://www.googleapis.com').post('/calendar/v3/calendars/primary/events', body => body.summary === writeArgs.title && body.attendees[0].email === 'guest@example.test' && body.guestsCanModify === false)
    .query(q => q.sendUpdates === 'none').reply(200, { id: 'created-1' });
  assert.deepEqual(await f.adapter.execute(call('create_event', writeArgs, { proposal: 'proposal-1' })), { status: 'completed', eventId: 'created-1' });
  assert.ok(mock.isDone());
  await assert.rejects(f.adapter.execute(call('create_event', writeArgs, { proposal: 'proposal-1' })));
});
test('unapproved, changed recipients and missing write scope dispatch nothing', async () => {
  const f = fixture(); f.connection.write = true;
  await assert.rejects(f.adapter.execute(call('create_event', writeArgs, { proposal: 'missing' })));
  f.proposals.set('p', { approved: true, digest: digest(writeArgs) });
  await assert.rejects(f.adapter.execute(call('create_event', { ...writeArgs, attendees: ['different@example.test'] }, { proposal: 'p' })));
  f.connection.write = false; await assert.rejects(f.adapter.execute(call('create_event', writeArgs, { proposal: 'p' })));
});
test('lost write response remains unknown; same proposal cannot dispatch again', async () => {
  const f = fixture(); f.connection.write = true; f.proposals.set('p', { approved: true, digest: digest(writeArgs) });
  let writes = 0;
  nock('https://www.googleapis.com').post('/calendar/v3/calendars/primary/events').query(true)
    .reply(() => { writes++; return [503, { error: { message: 'synthetic lost outcome' } }]; });
  assert.deepEqual(await f.adapter.execute(call('create_event', writeArgs, { proposal: 'p' })), { status: 'unknown' });
  await assert.rejects(f.adapter.execute(call('create_event', writeArgs, { proposal: 'p' })));
  assert.equal(writes, 1); assert.equal(f.outcomes[0].status, 'unknown');
});

test('inherited action names reject before authority claim', async () => {
  const f = fixture(); let claims = 0;
  f.port.claim = async () => { claims++; throw new Error('must not claim'); };
  for (const capability of ['constructor', 'toString', '__proto__']) await assert.rejects(f.adapter.execute(call(capability, readArgs)), /unreviewed action/);
  assert.equal(claims, 0);
});
test('caller mutation during async claim cannot change approved dispatch', async () => {
  const f = fixture(); f.connection.write = true;
  const original = structuredClone(writeArgs);
  f.proposals.set('p', { approved: true, digest: digest(original) });
  let release;
  const gate = new Promise(resolve => { release = resolve; });
  const claim = f.port.claim;
  f.port.claim = async c => { await gate; return claim(c); };
  nock('https://www.googleapis.com').post('/calendar/v3/calendars/primary/events', body => body.summary === 'Reviewed meeting' && body.attendees.length === 1 && body.attendees[0].email === 'guest@example.test')
    .query(q => q.sendUpdates === 'none').reply(200, { id: 'created-immutable' });
  const request = call('create_event', original, { proposal: 'p' });
  const pending = f.adapter.execute(request);
  original.attendees.push('unapproved@example.test');
  original.attendees[0] = 'replaced@example.test';
  original.title = 'Changed title';
  request.arguments = { ...original, send_notifications: 'all' };
  release();
  assert.deepEqual(await pending, { status: 'completed', eventId: 'created-immutable' });
});
