const module = await import("/pkg/polysig_webassembly_bindings.js");

// Initialize the webassembly
await module.default();

const { CggmpProtocol } = module;

const params = new URLSearchParams(document.location.search);
const pageData = JSON.parse(params.get('data'));
const { partyIndex, server, parameters, sessionIdSeed, keyShare, indices, message } = pageData;
const partyKeys = JSON.parse(params.get('keys'));

const publicKey = partyKeys[partyIndex].encrypt.public;

const participants = partyKeys.map((key) => {
  return key.encrypt.public;
});

const verifiers = partyKeys.map((key) => {
  return key.sign.public;
});

const signer = partyKeys[partyIndex].sign.private;
const options = {
  keypair: partyKeys[partyIndex].encrypt,
  server,
  parameters
};

const party = {
  publicKey,
  participants: indices.map((i) => participants[i]),
  isInitiator: partyIndex == 0,
  verifiers: indices.map((i) => verifiers[i]),
  partyIndex,
};

const protocol = new CggmpProtocol(options, keyShare);
const signature = await protocol.sign(
  party,
  sessionIdSeed,
  signer,
  message,
);

const el = document.querySelector("body");
el.innerHTML = `<p class="signature">${JSON.stringify({partyIndex, signature})}</p>`;
