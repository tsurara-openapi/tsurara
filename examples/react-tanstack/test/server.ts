import { setupServer } from 'msw/node';

/** The API of the tests: each test installs the generated handlers it needs (`server.use`). */
export const server = setupServer();
