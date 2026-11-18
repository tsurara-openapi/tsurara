import type { Session } from '../src/api';

/** The signed-in session of a request. A real app reads its cookie; the example has a fixed one. */
export const requestSession = async (): Promise<Session> => ({
  tenant: process.env.CARDS_TENANT ?? 'acme',
  token: process.env.CARDS_TOKEN ?? 'demo',
});
