import type { Attachment, Conversation } from "./types";

const DATABASE_NAME = "vlm_screenshot_action-data";
const STORE_NAME = "conversations";
const IMAGE_STORE_NAME = "images";
const RECORD_KEY = "all";

// Images used to live inline in the conversation record, so every debounced save
// rewrote every screenshot of every conversation. They now live in their own
// store keyed by attachment id, which makes a save cost one write per new image
// instead of one write per image ever captured.
const DATABASE_VERSION = 2;

// Screenshots are roughly a megabyte each; batching keeps a large first save
// (or a migrated legacy store) from becoming one oversized transaction.
const IMAGE_WRITE_BATCH = 16;

function openDatabase(): Promise<IDBDatabase> {
  return new Promise((resolve, reject) => {
    const request = indexedDB.open(DATABASE_NAME, DATABASE_VERSION);
    request.onupgradeneeded = () => {
      const database = request.result;
      if (!database.objectStoreNames.contains(STORE_NAME)) {
        database.createObjectStore(STORE_NAME);
      }
      if (!database.objectStoreNames.contains(IMAGE_STORE_NAME)) {
        database.createObjectStore(IMAGE_STORE_NAME);
      }
    };
    request.onsuccess = () => resolve(request.result);
    request.onerror = () => reject(request.error);
  });
}

export type ImageWrite = { id: string; dataUrl: string };

export type SavePlan = {
  /** Conversations with every image payload moved out, ready to persist. */
  stripped: Conversation[];
  /** Payloads that are not in the image store yet. */
  imagesToWrite: ImageWrite[];
  /** Every image id some stored conversation still refers to. */
  referencedImageIds: Set<string>;
};

/**
 * Splits a conversation list into the small record that gets persisted and the
 * image payloads that need writing. Pure, so the write plan can be tested
 * without IndexedDB.
 *
 * In-memory conversations are never mutated: the caller keeps rendering and
 * sending requests from the full data URLs.
 */
export function planConversationSave(
  conversations: Conversation[],
  persistedImageIds: ReadonlySet<string>,
): SavePlan {
  const imagesToWrite: ImageWrite[] = [];
  const queuedImageIds = new Set<string>();
  const referencedImageIds = new Set<string>();

  const stripped = conversations.map((conversation) => ({
    ...conversation,
    messages: conversation.messages.map((message) => {
      if (!message.attachments?.length) return message;
      return {
        ...message,
        attachments: message.attachments.map((attachment) => {
          referencedImageIds.add(attachment.id);
          if (!attachment.dataUrl) return attachment;
          if (!persistedImageIds.has(attachment.id) && !queuedImageIds.has(attachment.id)) {
            queuedImageIds.add(attachment.id);
            imagesToWrite.push({ id: attachment.id, dataUrl: attachment.dataUrl });
          }
          return { ...attachment, dataUrl: "" };
        }),
      };
    }),
  }));

  return { stripped, imagesToWrite, referencedImageIds };
}

/** Put every stored payload back, and drop attachments whose payload is gone. */
export function hydrateConversations(
  conversations: Conversation[],
  images: ReadonlyMap<string, string>,
): Conversation[] {
  return conversations.map((conversation) => ({
    ...conversation,
    messages: conversation.messages.map((message) => {
      if (!message.attachments?.length) return message;
      let changed = false;
      const attachments: Attachment[] = [];
      for (const attachment of message.attachments) {
        if (attachment.dataUrl) {
          attachments.push(attachment);
          continue;
        }
        const dataUrl = images.get(attachment.id);
        changed = true;
        if (dataUrl) attachments.push({ ...attachment, dataUrl });
      }
      return changed ? { ...message, attachments } : message;
    }),
  }));
}

// Ids known to be in the image store. Seeded by load, extended on a successful
// write, so a save only carries images that are genuinely new.
let persistedImageIds = new Set<string>();

function transactionDone(transaction: IDBTransaction): Promise<void> {
  return new Promise((resolve, reject) => {
    transaction.oncomplete = () => resolve();
    transaction.onerror = () => reject(transaction.error);
    transaction.onabort = () => reject(transaction.error);
  });
}

function readRecord(database: IDBDatabase): Promise<Conversation[]> {
  return new Promise((resolve, reject) => {
    const request = database.transaction(STORE_NAME, "readonly")
      .objectStore(STORE_NAME)
      .get(RECORD_KEY);
    request.onsuccess = () => resolve((request.result as Conversation[] | undefined) ?? []);
    request.onerror = () => reject(request.error);
  });
}

function readImages(database: IDBDatabase): Promise<Map<string, string>> {
  return new Promise((resolve, reject) => {
    const images = new Map<string, string>();
    const request = database.transaction(IMAGE_STORE_NAME, "readonly")
      .objectStore(IMAGE_STORE_NAME)
      .openCursor();
    request.onsuccess = () => {
      const cursor = request.result;
      if (!cursor) {
        resolve(images);
        return;
      }
      images.set(String(cursor.key), String(cursor.value));
      cursor.continue();
    };
    request.onerror = () => reject(request.error);
  });
}

export async function loadStoredConversations(): Promise<Conversation[]> {
  if (!("indexedDB" in window)) return [];
  const database = await openDatabase();
  try {
    const stored = await readRecord(database);
    const images = await readImages(database);
    // Anything already in the image store came from a previous save; anything
    // still inline is legacy data that the next save migrates.
    persistedImageIds = new Set(images.keys());
    return hydrateConversations(stored, images);
  } finally {
    database.close();
  }
}

export async function storeConversations(conversations: Conversation[]): Promise<void> {
  if (!("indexedDB" in window)) return;
  const database = await openDatabase();
  try {
    const plan = planConversationSave(conversations, persistedImageIds);
    // Write payloads first, in chunks. A large legacy conversation can hold
    // hundreds of megabytes, and one oversized transaction is likelier to fail
    // than several small ones. The record write below is the commit point, so a
    // failure here leaves the previously saved record untouched.
    for (let index = 0; index < plan.imagesToWrite.length; index += IMAGE_WRITE_BATCH) {
      const batch = plan.imagesToWrite.slice(index, index + IMAGE_WRITE_BATCH);
      const transaction = database.transaction(IMAGE_STORE_NAME, "readwrite");
      const done = transactionDone(transaction);
      const store = transaction.objectStore(IMAGE_STORE_NAME);
      for (const image of batch) store.put(image.dataUrl, image.id);
      await done;
      // Record progress as it lands, so a retry after a failure resumes here
      // instead of re-sending payloads that are already stored.
      for (const image of batch) persistedImageIds.add(image.id);
    }

    const transaction = database.transaction(
      [STORE_NAME, IMAGE_STORE_NAME],
      "readwrite",
    );
    const done = transactionDone(transaction);
    const imageStore = transaction.objectStore(IMAGE_STORE_NAME);
    transaction.objectStore(STORE_NAME).put(plan.stripped, RECORD_KEY);
    // Drop payloads no stored conversation refers to any more. Keys only, so
    // this stays cheap however large the images are.
    const cursorRequest = imageStore.openKeyCursor();
    cursorRequest.onsuccess = () => {
      const cursor = cursorRequest.result;
      if (!cursor) return;
      if (!plan.referencedImageIds.has(String(cursor.key))) {
        imageStore.delete(cursor.key);
      }
      cursor.continue();
    };
    await done;
    // Mirror what the store now holds: ids that are still referenced and known
    // to be stored. Without this the set would keep ids the sweep deleted and
    // skip them on a later save.
    const nextPersisted = new Set<string>();
    for (const id of persistedImageIds) {
      if (plan.referencedImageIds.has(id)) nextPersisted.add(id);
    }
    persistedImageIds = nextPersisted;
  } finally {
    database.close();
  }
}
