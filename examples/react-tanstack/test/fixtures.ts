import { getCardHandler } from '../src/generated/mocks/operations/getCard';
import { listCardsHandler } from '../src/generated/mocks/operations/listCards';
import type { Card } from '../src/generated/schemas/Card';

/** Cards per tenant: the bearer token is `<tenant>-token`. */
export const acmeOne: Card = { id: 'a1', title: 'Acme one', done: false };

export const cards: Record<string, Card[]> = {
  acme: [
    acmeOne,
    { id: 'a2', title: 'Acme two', done: false },
    { id: 'a3', title: 'Acme three', done: true },
  ],
  globex: [{ id: 'g1', title: 'Globex one', done: false }],
};

export const session = (tenant: string) => ({ tenant, token: `${tenant}-token` });

const tenantOf = (request: Request): string =>
  (request.headers.get('authorization') ?? '').replace(/^Bearer /, '').replace(/-token$/, '');

/** `listCards` a card per page, with the search applied, for the tenant of the token. */
export const listCards = (requests: string[] = []) =>
  listCardsHandler(({ request }) => {
    const url = new URL(request.url);
    requests.push(url.search);
    const q = url.searchParams.get('q') ?? '';
    const own = (cards[tenantOf(request)] ?? []).filter((c) => c.title.includes(q));
    const at = Number(url.searchParams.get('cursor') ?? '0');
    const next = at + 1 < own.length ? String(at + 1) : undefined;
    return {
      status: 200,
      body: { items: own.slice(at, at + 1), ...(next === undefined ? {} : { nextCursor: next }) },
    };
  });

/** `getCard` for the tenant of the token, 404 for others' cards. */
export const getCard = () =>
  getCardHandler(({ request, params }) => {
    const card = (cards[tenantOf(request)] ?? []).find((c) => c.id === params.cardId);
    return card ? { status: 200, body: card } : { status: 404, body: { title: 'No such card' } };
  });
