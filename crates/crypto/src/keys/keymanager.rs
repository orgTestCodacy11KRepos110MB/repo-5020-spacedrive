//! This module contains Spacedrive's key manager implementation.
//!
//! The key manager is used for keeping track of keys within memory, and mounting them on demand.
//!
//! The key manager is initialised, and added to a global state so it is accessible everywhere.
//! It is also populated with all keys from the Prisma database.
//!
//! # Examples
//!
//! ```rust
//! use sd_crypto::keys::keymanager::KeyManager;
//! use sd_crypto::Protected;
//! use sd_crypto::crypto::stream::Algorithm;
//! use sd_crypto::keys::hashing::{HashingAlgorithm, Params};
//!
//! let master_password = Protected::new(b"password".to_vec());
//!
//! // Initialise a `Keymanager` with no stored keys and no master password
//! let mut key_manager = KeyManager::new(vec![], None);
//!
//! // Set the master password
//! key_manager.set_master_password(master_password);
//!
//! let new_password = Protected::new(b"super secure".to_vec());
//!
//! // Register the new key with the key manager
//! let added_key = key_manager.add_to_keystore(new_password, Algorithm::XChaCha20Poly1305, HashingAlgorithm::Argon2id(Params::Standard)).unwrap();
//!
//! // Write the stored key to the database here (with `KeyManager::access_keystore()`)
//!
//! // Mount the key we just added (with the returned UUID)
//! key_manager.mount(added_key);
//!
//! // Retrieve all currently mounted, hashed keys to pass to a decryption function.
//! let keys = key_manager.enumerate_hashed_keys();
//! ```

use std::sync::Mutex;

use crate::crypto::stream::{StreamDecryption, StreamEncryption};
use crate::primitives::{
	derive_key, generate_master_key, generate_nonce, generate_salt, to_array, OnboardingConfig,
	KEY_LEN, LATEST_STORED_KEY, MASTER_PASSWORD_CONTEXT, ROOT_KEY_CONTEXT,
};
use crate::{
	crypto::stream::Algorithm,
	primitives::{ENCRYPTED_KEY_LEN, SALT_LEN},
	Protected,
};
use crate::{Error, Result};

use dashmap::{DashMap, DashSet};
use uuid::Uuid;

#[cfg(feature = "serde")]
use serde_big_array::BigArray;

use super::hashing::HashingAlgorithm;

/// This is a stored key, and can be freely written to Prisma/another database.
#[derive(Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "rspc", derive(specta::Type))]
pub struct StoredKey {
	pub uuid: uuid::Uuid, // uuid for identification. shared with mounted keys
	pub version: StoredKeyVersion,
	pub algorithm: Algorithm, // encryption algorithm for encrypting the master key. can be changed (requires a re-encryption though)
	pub hashing_algorithm: HashingAlgorithm, // hashing algorithm used for hashing the key with the content salt
	pub content_salt: [u8; SALT_LEN],
	#[cfg_attr(feature = "serde", serde(with = "BigArray"))] // salt used for file data
	pub master_key: [u8; ENCRYPTED_KEY_LEN], // this is for encrypting the `key`
	pub master_key_nonce: Vec<u8>, // nonce for encrypting the master key
	pub key_nonce: Vec<u8>,        // nonce used for encrypting the main key
	pub key: Vec<u8>, // encrypted. the key stored in spacedrive (e.g. generated 64 char key)
	pub salt: [u8; SALT_LEN],
	pub memory_only: bool,
	pub automount: bool,
}

#[derive(Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "rspc", derive(specta::Type))]
pub enum StoredKeyVersion {
	V1,
}

/// This is a mounted key, and needs to be kept somewhat hidden.
///
/// This contains the plaintext key, and the same key hashed with the content salt.
#[derive(Clone)]
pub struct MountedKey {
	pub uuid: Uuid, // used for identification. shared with stored keys
	pub hashed_key: Protected<[u8; KEY_LEN]>, // this is hashed with the content salt, for instant access
}

/// This is the key manager itself.
///
/// It contains the keystore, the keymount, the master password and the default key.
///
/// Use the associated functions to interact with it.
pub struct KeyManager {
	root_key: Mutex<Option<Protected<[u8; KEY_LEN]>>>, // the root key for the vault
	verification_key: Mutex<Option<StoredKey>>,
	keystore: DashMap<Uuid, StoredKey>,
	keymount: DashMap<Uuid, MountedKey>,
	default: Mutex<Option<Uuid>>,
	mounting_queue: DashSet<Uuid>,
}

/// The `KeyManager` functions should be used for all key-related management.
impl KeyManager {
	/// Initialize the Key Manager with `StoredKeys` retrieved from Prisma
	pub fn new(stored_keys: Vec<StoredKey>) -> Result<Self> {
		let keymanager = Self {
			root_key: Mutex::new(None),
			verification_key: Mutex::new(None),
			keystore: DashMap::new(),
			keymount: DashMap::new(),
			default: Mutex::new(None),
			mounting_queue: DashSet::new(),
		};

		keymanager.populate_keystore(stored_keys)?;

		Ok(keymanager)
	}

	/// This should be used to generate everything for the user during onboarding.
	///
	/// This will create a master password (a 7-word diceware passphrase), and a secret key (16 bytes, hex encoded)
	///
	/// It will also generate a verification key, which should be written to the database.
	#[allow(clippy::needless_pass_by_value)]
	pub fn onboarding(config: OnboardingConfig) -> Result<StoredKey> {
		let content_salt = generate_salt();
		let secret_key = config.secret_key.map(Self::convert_secret_key_string);

		let algorithm = config.algorithm;
		let hashing_algorithm = config.hashing_algorithm;

		// Hash the master password
		let hashed_password = hashing_algorithm.hash(
			Protected::new(config.password.expose().as_bytes().to_vec()),
			content_salt,
			secret_key,
		)?;

		let salt = generate_salt();
		let uuid = uuid::Uuid::nil();

		// Generate items we'll need for encryption
		let master_key = generate_master_key();
		let master_key_nonce = generate_nonce(algorithm);

		let root_key = generate_master_key();
		let root_key_nonce = generate_nonce(algorithm);

		// Encrypt the master key with the hashed master password
		let encrypted_master_key = to_array::<ENCRYPTED_KEY_LEN>(StreamEncryption::encrypt_bytes(
			derive_key(hashed_password, salt, MASTER_PASSWORD_CONTEXT),
			&master_key_nonce,
			algorithm,
			master_key.expose(),
			&[],
		)?)?;

		let encrypted_root_key = StreamEncryption::encrypt_bytes(
			master_key,
			&root_key_nonce,
			algorithm,
			root_key.expose(),
			&[],
		)?;

		let verification_key = StoredKey {
			uuid,
			version: LATEST_STORED_KEY,
			algorithm,
			hashing_algorithm,
			content_salt, // salt used for hashing
			master_key: encrypted_master_key,
			master_key_nonce,
			key_nonce: root_key_nonce,
			key: encrypted_root_key,
			salt, // salt used for key derivation
			memory_only: false,
			automount: false,
		};

		Ok(verification_key)
	}

	/// This function should be used to populate the keystore with multiple stored keys at a time.
	///
	/// It's suitable for when you created the key manager without populating it.
	///
	/// This also detects the nil-UUID master passphrase verification key
	pub fn populate_keystore(&self, stored_keys: Vec<StoredKey>) -> Result<()> {
		for key in stored_keys {
			if self.keystore.contains_key(&key.uuid) {
				continue;
			}

			if key.uuid.is_nil() {
				*self.verification_key.lock()? = Some(key);
			} else {
				self.keystore.insert(key.uuid, key);
			}
		}

		Ok(())
	}

	/// This function removes a key from the keystore, the keymount and it's unset as the default.
	pub fn remove_key(&self, uuid: Uuid) -> Result<()> {
		if self.keystore.contains_key(&uuid) {
			// if key is default, clear it
			// do this manually to prevent deadlocks
			let mut default = self.default.lock()?;
			if *default == Some(uuid) {
				*default = None;
			}
			drop(default);

			// unmount if mounted
			self.keymount
				.contains_key(&uuid)
				.then(|| self.keymount.remove(&uuid));

			// remove from keystore
			self.keystore.remove(&uuid);
		}

		Ok(())
	}

	#[allow(clippy::needless_pass_by_value)]
	pub fn change_master_password(
		&self,
		master_password: Protected<String>,
		algorithm: Algorithm,
		hashing_algorithm: HashingAlgorithm,
		secret_key: Option<Protected<String>>,
	) -> Result<StoredKey> {
		let secret_key = secret_key.map(Self::convert_secret_key_string);
		let content_salt = generate_salt();

		let hashed_password = hashing_algorithm.hash(
			Protected::new(master_password.expose().as_bytes().to_vec()),
			content_salt,
			secret_key,
		)?;

		let uuid = uuid::Uuid::nil();

		// Generate items we'll need for encryption
		let master_key = generate_master_key();
		let master_key_nonce = generate_nonce(algorithm);

		let root_key = self.get_root_key()?;
		let root_key_nonce = generate_nonce(algorithm);

		let salt = generate_salt();

		// Encrypt the master key with the hashed master password
		let encrypted_master_key = to_array::<ENCRYPTED_KEY_LEN>(StreamEncryption::encrypt_bytes(
			derive_key(hashed_password, salt, MASTER_PASSWORD_CONTEXT),
			&master_key_nonce,
			algorithm,
			master_key.expose(),
			&[],
		)?)?;

		let encrypted_root_key = StreamEncryption::encrypt_bytes(
			master_key,
			&root_key_nonce,
			algorithm,
			root_key.expose(),
			&[],
		)?;

		let verification_key = StoredKey {
			uuid,
			version: LATEST_STORED_KEY,
			algorithm,
			hashing_algorithm,
			content_salt,
			master_key: encrypted_master_key,
			master_key_nonce,
			key_nonce: root_key_nonce,
			key: encrypted_root_key,
			salt,
			memory_only: false,
			automount: false,
		};

		*self.verification_key.lock()? = Some(verification_key.clone());

		Ok(verification_key)
	}

	/// This re-encrypts master keys so they can be imported from a key backup into the current key manager.
	///
	/// It returns a `Vec<StoredKey>` so they can be written to Prisma
	#[allow(clippy::needless_pass_by_value)]
	pub fn import_keystore_backup(
		&self,
		master_password: Protected<String>,    // at the time of the backup
		secret_key: Option<Protected<String>>, // at the time of the backup
		stored_keys: &[StoredKey],             // from the backup
	) -> Result<Vec<StoredKey>> {
		// this backup should contain a verification key, which will tell us the algorithm+hashing algorithm
		let secret_key = secret_key.map(Self::convert_secret_key_string);

		let mut old_verification_key = None;

		let keys: Vec<StoredKey> = stored_keys
			.iter()
			.filter_map(|key| {
				if key.uuid.is_nil() {
					old_verification_key = Some(key.clone());
					None
				} else {
					Some(key.clone())
				}
			})
			.collect();

		let old_verification_key = old_verification_key.ok_or(Error::NoVerificationKey)?;

		let old_root_key = match old_verification_key.version {
			StoredKeyVersion::V1 => {
				let hashed_password = old_verification_key.hashing_algorithm.hash(
					Protected::new(master_password.expose().as_bytes().to_vec()),
					old_verification_key.content_salt,
					secret_key,
				)?;

				// decrypt the root key's KEK
				let master_key = StreamDecryption::decrypt_bytes(
					derive_key(
						hashed_password,
						old_verification_key.salt,
						MASTER_PASSWORD_CONTEXT,
					),
					&old_verification_key.master_key_nonce,
					old_verification_key.algorithm,
					&old_verification_key.master_key,
					&[],
				)?;

				// get the root key from the backup
				let old_root_key = StreamDecryption::decrypt_bytes(
					Protected::new(to_array(master_key.into_inner())?),
					&old_verification_key.key_nonce,
					old_verification_key.algorithm,
					&old_verification_key.key,
					&[],
				)?;

				Protected::new(to_array(old_root_key.into_inner())?)
			}
		};

		let mut reencrypted_keys = Vec::new();

		for key in keys {
			if self.keystore.contains_key(&key.uuid) {
				continue;
			}

			match key.version {
				StoredKeyVersion::V1 => {
					// decrypt the key's master key
					let master_key = StreamDecryption::decrypt_bytes(
						derive_key(old_root_key.clone(), key.salt, ROOT_KEY_CONTEXT),
						&key.master_key_nonce,
						key.algorithm,
						&key.master_key,
						&[],
					)
					.map_or(Err(Error::IncorrectPassword), |v| {
						Ok(Protected::new(to_array::<KEY_LEN>(v.into_inner())?))
					})?;

					// generate a new nonce
					let master_key_nonce = generate_nonce(key.algorithm);

					let salt = generate_salt();

					// encrypt the master key with the current root key
					let encrypted_master_key = to_array(StreamEncryption::encrypt_bytes(
						derive_key(self.get_root_key()?, salt, ROOT_KEY_CONTEXT),
						&master_key_nonce,
						key.algorithm,
						master_key.expose(),
						&[],
					)?)?;

					let mut updated_key = key.clone();
					updated_key.master_key_nonce = master_key_nonce;
					updated_key.master_key = encrypted_master_key;
					updated_key.salt = salt;

					reencrypted_keys.push(updated_key.clone());
					self.keystore.insert(updated_key.uuid, updated_key);
				}
			}
		}

		Ok(reencrypted_keys)
	}

	/// This requires both the master password and the secret key
	///
	/// The master password and secret key are hashed together.
	/// This minimises the risk of an attacker obtaining the master password, as both of these are required to unlock the vault (and both should be stored separately).
	///
	/// Both values need to be correct, otherwise this function will return a generic error.
	///
	/// The invalidate function is to handle query invalidation, so that the UI updates correctly. Leave it blank if this isn't required.
	///
	/// Note: The invalidation function is ran after updating the queue both times, so it isn't required externally.
	#[allow(clippy::needless_pass_by_value)]
	pub fn set_master_password<F>(
		&self,
		master_password: Protected<String>,
		secret_key: Option<Protected<String>>,
		invalidate: F,
	) -> Result<()>
	where
		F: Fn(),
	{
		let uuid = Uuid::nil();

		if self.has_master_password()? {
			return Err(Error::KeyAlreadyMounted);
		} else if self.is_queued(uuid) {
			return Err(Error::KeyAlreadyQueued);
		}

		let verification_key = (*self.verification_key.lock()?)
			.as_ref()
			.map_or(Err(Error::NoVerificationKey), |k| Ok(k.clone()))?;

		let secret_key = secret_key.map(Self::convert_secret_key_string);

		self.mounting_queue.insert(uuid);
		invalidate();

		match verification_key.version {
			StoredKeyVersion::V1 => {
				let hashed_password = verification_key
					.hashing_algorithm
					.hash(
						Protected::new(master_password.expose().as_bytes().to_vec()),
						verification_key.content_salt,
						secret_key,
					)
					.map_err(|e| {
						self.remove_from_queue(uuid).ok();
						e
					})?;

				let master_key = StreamDecryption::decrypt_bytes(
					derive_key(
						hashed_password,
						verification_key.salt,
						MASTER_PASSWORD_CONTEXT,
					),
					&verification_key.master_key_nonce,
					verification_key.algorithm,
					&verification_key.master_key,
					&[],
				)
				.map_err(|_| {
					self.remove_from_queue(uuid).ok();
					Error::IncorrectKeymanagerDetails
				})?;

				*self.root_key.lock()? = Some(Protected::new(
					to_array(
						StreamDecryption::decrypt_bytes(
							Protected::new(to_array(master_key.into_inner())?),
							&verification_key.key_nonce,
							verification_key.algorithm,
							&verification_key.key,
							&[],
						)?
						.expose()
						.clone(),
					)
					.map_err(|e| {
						self.remove_from_queue(uuid).ok();
						e
					})?,
				));

				self.remove_from_queue(uuid)?;
			}
		}

		invalidate();

		Ok(())
	}

	/// This function does not return a value by design.
	///
	/// Once a key is mounted, access it with `KeyManager::access()`
	///
	/// This is to ensure that only functions which require access to the mounted key receive it.
	///
	/// We could add a log to this, so that the user can view mounts
	pub fn mount(&self, uuid: Uuid) -> Result<()> {
		if self.keymount.get(&uuid).is_some() {
			return Err(Error::KeyAlreadyMounted);
		} else if self.is_queued(uuid) {
			return Err(Error::KeyAlreadyQueued);
		}

		self.keystore
			.get(&uuid)
			.map_or(Err(Error::KeyNotFound), |stored_key| {
				match stored_key.version {
					StoredKeyVersion::V1 => {
						self.mounting_queue.insert(uuid);

						let master_key = StreamDecryption::decrypt_bytes(
							derive_key(self.get_root_key()?, stored_key.salt, ROOT_KEY_CONTEXT),
							&stored_key.master_key_nonce,
							stored_key.algorithm,
							&stored_key.master_key,
							&[],
						)
						.map_or_else(
							|_| {
								self.remove_from_queue(uuid).ok();
								Err(Error::IncorrectPassword)
							},
							|v| Ok(Protected::new(to_array(v.into_inner())?)),
						)?;
						// Decrypt the StoredKey using the decrypted master key
						let key = StreamDecryption::decrypt_bytes(
							master_key,
							&stored_key.key_nonce,
							stored_key.algorithm,
							&stored_key.key,
							&[],
						)
						.map_err(|e| {
							self.remove_from_queue(uuid).ok();
							e
						})?;

						// Hash the key once with the parameters/algorithm the user selected during first mount
						let hashed_key = stored_key
							.hashing_algorithm
							.hash(key, stored_key.content_salt, None)
							.map_err(|e| {
								self.remove_from_queue(uuid).ok();
								e
							})?;

						self.keymount.insert(
							uuid,
							MountedKey {
								uuid: stored_key.uuid,
								hashed_key,
							},
						);

						self.remove_from_queue(uuid)?;
					}
				}

				Ok(())
			})
	}

	/// This function is used for getting the key value itself, from a given UUID.
	///
	/// The master password/salt needs to be present, so we are able to decrypt the key itself from the stored key.
	pub fn get_key(&self, uuid: Uuid) -> Result<Protected<Vec<u8>>> {
		self.keystore
			.get(&uuid)
			.map_or(Err(Error::KeyNotFound), |stored_key| {
				let master_key = StreamDecryption::decrypt_bytes(
					derive_key(self.get_root_key()?, stored_key.salt, ROOT_KEY_CONTEXT),
					&stored_key.master_key_nonce,
					stored_key.algorithm,
					&stored_key.master_key,
					&[],
				)
				.map_or(Err(Error::IncorrectPassword), |k| {
					Ok(Protected::new(to_array(k.into_inner())?))
				})?;

				// Decrypt the StoredKey using the decrypted master key
				let key = StreamDecryption::decrypt_bytes(
					master_key,
					&stored_key.key_nonce,
					stored_key.algorithm,
					&stored_key.key,
					&[],
				)?;

				Ok(key)
			})
	}

	/// This function is used to add a new key/password to the keystore.
	///
	/// You should use this when a new key is added, as it will generate salts/nonces/etc.
	///
	/// It does not mount the key, it just registers it.
	///
	/// Once added, you will need to use `KeyManager::access_keystore()` to retrieve it and add it to Prisma.
	///
	/// You may use the returned ID to identify this key.
	///
	/// You may optionally provide a content salt, if not one will be generated (used primarily for password-based decryption)
	#[allow(clippy::needless_pass_by_value)]
	pub fn add_to_keystore(
		&self,
		key: Protected<Vec<u8>>,
		algorithm: Algorithm,
		hashing_algorithm: HashingAlgorithm,
		memory_only: bool,
		automount: bool,
		content_salt: Option<[u8; SALT_LEN]>,
	) -> Result<Uuid> {
		let uuid = uuid::Uuid::new_v4();

		// Generate items we'll need for encryption
		let key_nonce = generate_nonce(algorithm);
		let master_key = generate_master_key();
		let master_key_nonce = generate_nonce(algorithm);

		let content_salt = content_salt.map_or_else(generate_salt, |v| v);

		// salt used for the kdf
		let salt = generate_salt();

		// Encrypt the master key with a derived key (derived from the root key)
		let encrypted_master_key = to_array::<ENCRYPTED_KEY_LEN>(StreamEncryption::encrypt_bytes(
			derive_key(self.get_root_key()?, salt, ROOT_KEY_CONTEXT),
			&master_key_nonce,
			algorithm,
			master_key.expose(),
			&[],
		)?)?;

		// Encrypt the actual key (e.g. user-added/autogenerated, text-encodable)
		let encrypted_key =
			StreamEncryption::encrypt_bytes(master_key, &key_nonce, algorithm, &key, &[])?;

		// Insert it into the Keystore
		self.keystore.insert(
			uuid,
			StoredKey {
				uuid,
				version: LATEST_STORED_KEY,
				algorithm,
				hashing_algorithm,
				content_salt,
				master_key: encrypted_master_key,
				master_key_nonce,
				key_nonce,
				key: encrypted_key,
				salt,
				memory_only,
				automount,
			},
		);

		// Return the ID so it can be identified
		Ok(uuid)
	}

	#[allow(clippy::needless_pass_by_value)]
	fn convert_secret_key_string(secret_key: Protected<String>) -> Protected<Vec<u8>> {
		Protected::new(secret_key.expose().as_bytes().to_vec())
	}

	/// This function is for accessing the internal keymount.
	///
	/// We could add a log to this, so that the user can view accesses
	pub fn access_keymount(&self, uuid: Uuid) -> Result<MountedKey> {
		self.keymount
			.get(&uuid)
			.map_or(Err(Error::KeyNotFound), |v| Ok(v.clone()))
	}

	/// This function is for accessing a `StoredKey`.
	pub fn access_keystore(&self, uuid: Uuid) -> Result<StoredKey> {
		self.keystore
			.get(&uuid)
			.map_or(Err(Error::KeyNotFound), |v| Ok(v.clone()))
	}

	/// This allows you to set the default key
	pub fn set_default(&self, uuid: Uuid) -> Result<()> {
		if self.keystore.contains_key(&uuid) {
			*self.default.lock()? = Some(uuid);
			Ok(())
		} else {
			Err(Error::KeyNotFound)
		}
	}

	/// This allows you to get the default key's ID
	pub fn get_default(&self) -> Result<Uuid> {
		self.default.lock()?.ok_or(Error::NoDefaultKeySet)
	}

	/// This should ONLY be used internally.
	fn get_root_key(&self) -> Result<Protected<[u8; KEY_LEN]>> {
		self.root_key.lock()?.clone().ok_or(Error::NoMasterPassword)
	}

	pub fn get_verification_key(&self) -> Result<StoredKey> {
		self.verification_key
			.lock()?
			.clone()
			.ok_or(Error::NoVerificationKey)
	}

	pub fn is_memory_only(&self, uuid: Uuid) -> Result<bool> {
		self.keystore
			.get(&uuid)
			.map_or(Err(Error::KeyNotFound), |v| Ok(v.memory_only))
	}

	pub fn change_automount_status(&self, uuid: Uuid, status: bool) -> Result<()> {
		let updated_key = self
			.keystore
			.get(&uuid)
			.map_or(Err(Error::KeyNotFound), |v| {
				let mut updated_key = v.clone();
				updated_key.automount = status;
				Ok(updated_key)
			})?;

		self.keystore.remove(&uuid);
		self.keystore.insert(uuid, updated_key);
		Ok(())
	}

	/// This function is for getting an entire collection of hashed keys.
	///
	/// These are ideal for passing over to decryption functions, as each decryption attempt is negligible, performance wise.
	///
	/// This means we don't need to keep super specific track of which key goes to which file, and we can just throw all of them at it.
	#[must_use]
	pub fn enumerate_hashed_keys(&self) -> Vec<Protected<[u8; KEY_LEN]>> {
		self.keymount
			.iter()
			.map(|mounted_key| mounted_key.hashed_key.clone())
			.collect::<Vec<Protected<[u8; KEY_LEN]>>>()
	}

	/// This function is for converting a memory-only key to a saved key which syncs to the library.
	///
	/// The returned value needs to be written to the database.
	pub fn sync_to_database(&self, uuid: Uuid) -> Result<StoredKey> {
		if !self.is_memory_only(uuid)? {
			return Err(Error::KeyNotMemoryOnly);
		}

		let updated_key = self
			.keystore
			.get(&uuid)
			.map_or(Err(Error::KeyNotFound), |v| {
				let mut updated_key = v.clone();
				updated_key.memory_only = false;
				Ok(updated_key)
			})?;

		self.keystore.remove(&uuid);
		self.keystore.insert(uuid, updated_key.clone());

		Ok(updated_key)
	}

	/// This function is for removing a previously-added master password
	pub fn clear_root_key(&self) -> Result<()> {
		*self.root_key.lock()? = None;

		Ok(())
	}

	/// This function is used for seeing if the key manager has a master password.
	///
	/// Technically this checks for the root key, but it makes no difference to the front end.
	pub fn has_master_password(&self) -> Result<bool> {
		Ok(self.root_key.lock()?.is_some())
	}

	/// This function is used for unmounting all keys at once.
	pub fn empty_keymount(&self) {
		// i'm unsure whether or not `.clear()` also calls drop
		// if it doesn't, we're going to need to find another way to call drop on these values
		// that way they will be zeroized and removed from memory fully
		self.keymount.clear();
	}

	/// This function is for unmounting a key from the key manager
	///
	/// This does not remove the key from the key store
	pub fn unmount(&self, uuid: Uuid) -> Result<()> {
		self.keymount.remove(&uuid).ok_or(Error::KeyNotMounted)?;

		Ok(())
	}

	/// This function returns a Vec of `StoredKey`s, so you can write them somewhere/update the database with them/etc
	///
	/// The database and keystore should be in sync at ALL times (unless the user chose an in-memory only key)
	#[must_use]
	pub fn dump_keystore(&self) -> Vec<StoredKey> {
		self.keystore.iter().map(|key| key.clone()).collect()
	}

	#[must_use]
	pub fn get_mounted_uuids(&self) -> Vec<Uuid> {
		self.keymount.iter().map(|key| key.uuid).collect()
	}

	pub fn get_queue(&self) -> Vec<Uuid> {
		self.mounting_queue.iter().map(|u| *u).collect()
	}

	pub fn is_queued(&self, uuid: Uuid) -> bool {
		self.mounting_queue.contains(&uuid)
	}

	pub fn remove_from_queue(&self, uuid: Uuid) -> Result<()> {
		self.mounting_queue
			.remove(&uuid)
			.ok_or(Error::KeyNotQueued)?;

		Ok(())
	}
}
