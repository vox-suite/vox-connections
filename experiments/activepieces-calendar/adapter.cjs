'use strict';
const { createHash } = require('node:crypto');
const { googleCalendar } = require('@activepieces/piece-google-calendar');

const VERSION = '0.12.0';
const CONTRACT = Object.freeze({
  list_events: { action: 'google_calendar_list_events', effect: 'read' },
  create_event: { action: 'google_calendar_create_event', effect: 'write' },
});
// This is a local evaluator port, not an authenticated host interface. Production
// must supply Connections' durable claim/current-authority guard, never agent flags.
function canonical(value) {
  if (Array.isArray(value)) return `[${value.map(canonical).join(',')}]`;
  if (value && typeof value === 'object') return `{${Object.keys(value).sort().map(k => `${JSON.stringify(k)}:${canonical(value[k])}`).join(',')}}`;
  return JSON.stringify(value);
}
function digest(value) { return createHash('sha256').update(canonical(value)).digest('hex'); }
function boundedText(value, limit) { return typeof value === 'string' && value.length > 0 && Buffer.byteLength(value) <= limit; }
function reviewedInput(capability, input) {
  if (!input || Object.getPrototypeOf(input) !== Object.prototype) throw new Error('invalid arguments');
  const allowed = capability === 'list_events'
    ? ['calendar_id', 'start_date', 'end_date']
    : ['calendar_id', 'title', 'start_date_time', 'end_date_time', 'attendees', 'send_notifications'];
  if (Object.keys(input).some(k => !allowed.includes(k))) throw new Error('unreviewed field');
  if (!boundedText(input.calendar_id, 256) || /[/?#]/.test(input.calendar_id)) throw new Error('invalid calendar');
  const dates = capability === 'list_events' ? ['start_date', 'end_date'] : ['start_date_time', 'end_date_time'];
  for (const name of dates) {
    if (typeof input[name] !== 'string' || !/T.*(?:Z|[+-]\d\d:\d\d)$/.test(input[name]) || !Number.isFinite(Date.parse(input[name]))) throw new Error('explicit zoned date required');
  }
  if (Date.parse(input[dates[1]]) <= Date.parse(input[dates[0]])) throw new Error('invalid date range');
  if (capability === 'list_events') return { ...input, event_types: [], singleEvents: true };
  if (!boundedText(input.title, 512) || !Array.isArray(input.attendees) || input.attendees.length > 10
    || input.attendees.some(e => typeof e !== 'string' || !/^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(e))
    || !['none', 'all'].includes(input.send_notifications)) throw new Error('invalid event');
  return { ...input, guests_can_modify: false, guests_can_invite_others: false, guests_can_see_other_guests: false, create_meet_link: false };
}
class CalendarPieceAdapter {
  constructor(authorityPort) { this.authority = authorityPort; }
  async execute(call) {
    if (!Object.hasOwn(CONTRACT, call.capability)) throw new Error('unreviewed action/version');
    const contract = CONTRACT[call.capability];
    if (call.pieceVersion !== VERSION) throw new Error('unreviewed action/version');
    const snapshotArguments = structuredClone(call.arguments);
    const propsValue = reviewedInput(call.capability, snapshotArguments);
    if (Array.isArray(snapshotArguments.attendees)) Object.freeze(snapshotArguments.attendees);
    Object.freeze(snapshotArguments);
    call = Object.freeze({ ...call, arguments: snapshotArguments });
    // Port verifies context+agent+connection+current grant+service scope and, for
    // writes, an authenticated exact proposal decision plus a durable single claim.
    const auth = await this.authority.claim({ ...call, effect: contract.effect, argumentDigest: digest(call.arguments) });
    let result;
    try {
      result = await googleCalendar._actions[contract.action].run({ auth, propsValue });
    } catch (_) {
      await this.authority.finish(call, contract.effect === 'write' ? 'unknown' : 'failed');
      return { status: contract.effect === 'write' ? 'unknown' : 'failed' };
    }
    if (contract.effect === 'read') {
      const body = result.body;
      if (!body || !Array.isArray(body.items) || body.items.length > 250 || Buffer.byteLength(JSON.stringify(body)) > 1024 * 1024) {
        await this.authority.finish(call, 'failed'); return { status: 'failed' };
      }
      await this.authority.finish(call, 'completed');
      // AP's action exposes Google's first page only; never imply a total count.
      return { status: 'completed', events: body.items.map(e => ({ id: e.id, summary: e.summary, start: e.start, end: e.end })), incomplete: Boolean(body.nextPageToken) };
    }
    if (!result || typeof result.id !== 'string') {
      await this.authority.finish(call, 'unknown'); return { status: 'unknown' };
    }
    await this.authority.finish(call, 'completed');
    return { status: 'completed', eventId: result.id };
  }
}
module.exports = { CalendarPieceAdapter, digest, VERSION };
