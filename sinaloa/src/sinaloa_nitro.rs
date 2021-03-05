//! Nitro-Enclave-specific material for Sinaloa
//!
//! ## Authors
//!
//! The Veracruz Development Team.
//!
//! ## Licensing and copyright notice
//!
//! See the `LICENSE.markdown` file in the Veracruz root directory for
//! information on licensing and copyright.

#[cfg(feature = "nitro")]
pub mod sinaloa_nitro {
    use crate::ec2_instance::EC2Instance;
    use crate::sinaloa::Sinaloa;
    use crate::sinaloa::SinaloaError;
    use lazy_static::lazy_static;
    use std::sync::Mutex;
    use veracruz_utils::{
        policy::EnclavePlatform, RuntimeManagerMessage, NitroEnclave, NitroError, NitroStatus,
    };

    const RUNTIME_MANAGER_EIF_PATH: &str = "../runtime-manager/runtime_manager.eif";
    const NITRO_ROOT_ENCLAVE_EIF_PATH: &str = "../nitro-root-enclave/nitro_root_enclave.eif";
    const NITRO_ROOT_ENCLAVE_SERVER_PATH: &str =
        "../nitro-root-enclave-server/target/debug/nitro-root-enclave-server";

    lazy_static! {
        //static ref NRE_CONTEXT: Mutex<Option<NitroEnclave>> = Mutex::new(None);
        static ref NRE_CONTEXT: Mutex<Option<EC2Instance>> = Mutex::new(None);
    }

    pub struct SinaloaNitro {
        enclave: NitroEnclave,
    }

    impl Sinaloa for SinaloaNitro {
        fn new(policy_json: &str) -> Result<Self, SinaloaError> {
            // Set up, initialize Nitro Root Enclave
            let policy: veracruz_utils::VeracruzPolicy =
                veracruz_utils::VeracruzPolicy::from_json(policy_json)?;

            {
                let mut nre_guard = NRE_CONTEXT.lock()?;
                if nre_guard.is_none() {
                    println!("NITRO ROOT ENCLAVE IS UNINITIALIZED.");
                    let runtime_manager_hash = policy
                        .runtime_manager_hash(&EnclavePlatform::Nitro)
                        .map_err(|err| SinaloaError::VeracruzUtilError(err))?;
                    let nre_context =
                        SinaloaNitro::native_attestation(&policy.proxy_attestation_server_url(), &runtime_manager_hash)?;
                    *nre_guard = Some(nre_context);
                }
            }

            println!("SinaloaNitro::new native_attestation complete. instantiating Runtime Manager");
            #[cfg(feature = "debug")]
            let runtime_manager_enclave = {
                println!("Starting Runtime Manager enclave in debug mode");
                NitroEnclave::new(
                    false,
                    RUNTIME_MANAGER_EIF_PATH,
                    true,
                    Some(SinaloaNitro::sinaloa_ocall_handler),
                )
                .map_err(|err| SinaloaError::NitroError(err))?
            };
            #[cfg(not(feature = "debug"))]
            let runtime_manager_enclave = {
                println!("Starting Runtime Manager enclave in release mode");
                NitroEnclave::new(
                    false,
                    RUNTIME_MANAGER_EIF_PATH,
                    false,
                    Some(SinaloaNitro::sinaloa_ocall_handler),
                )
                .map_err(|err| SinaloaError::NitroError(err))?
            };
            println!("SinaloaNitro::new NitroEnclave::new returned");
            let meta = Self {
                enclave: runtime_manager_enclave,
            };
            println!("SinaloaNitro::new Runtime Manager instantiated. Calling initialize");
            std::thread::sleep(std::time::Duration::from_millis(10000));

            let initialize: RuntimeManagerMessage = RuntimeManagerMessage::Initialize(policy_json.to_string());

            let encoded_buffer: Vec<u8> = bincode::serialize(&initialize)?;
            meta.enclave.send_buffer(&encoded_buffer)?;

            // read the response
            let status_buffer = meta.enclave.receive_buffer()?;

            let message: RuntimeManagerMessage = bincode::deserialize(&status_buffer[..])?;
            let status = match message {
                RuntimeManagerMessage::Status(status) => status,
                _ => return Err(SinaloaError::RuntimeManagerMessageStatus(message)),
            };
            match status {
                NitroStatus::Success => (),
                _ => return Err(SinaloaError::NitroStatus(status)),
            }
            println!("SinaloaNitro::new complete. Returning");
            return Ok(meta);
        }

        fn plaintext_data(&self, data: Vec<u8>) -> Result<Option<Vec<u8>>, SinaloaError> {
            let parsed = transport_protocol::parse_runtime_manager_request(&data)?;

            if parsed.has_request_proxy_psa_attestation_token() {
                let rpat = parsed.get_request_proxy_psa_attestation_token();
                let challenge = transport_protocol::parse_request_proxy_psa_attestation_token(rpat);
                let (psa_attestation_token, pubkey, device_id) =
                    self.proxy_psa_attestation_get_token(challenge)?;
                let serialized_pat = transport_protocol::serialize_proxy_psa_attestation_token(
                    &psa_attestation_token,
                    &pubkey,
                    device_id,
                )?;
                Ok(Some(serialized_pat))
            } else {
                return Err(SinaloaError::InvalidProtoBufMessage);
            }
        }

        // Note: this function will go away
        fn get_enclave_cert(&self) -> Result<Vec<u8>, SinaloaError> {
            let certificate = {
                let message = RuntimeManagerMessage::GetEnclaveCert;
                let message_buffer = bincode::serialize(&message)?;
                self.enclave.send_buffer(&message_buffer)?;
                // Read the resulting data as the certificate
                let received_buffer = self.enclave.receive_buffer()?;
                let received_message: RuntimeManagerMessage = bincode::deserialize(&received_buffer)?;
                match received_message {
                    RuntimeManagerMessage::EnclaveCert(cert) => cert,
                    _ => return Err(SinaloaError::InvalidRuntimeManagerMessage(received_message))?,
                }
            };
            return Ok(certificate);
        }

        // Note: This function will go away
        fn get_enclave_name(&self) -> Result<String, SinaloaError> {
            let name: String = {
                let message = RuntimeManagerMessage::GetEnclaveName;
                let message_buffer = bincode::serialize(&message)?;
                self.enclave.send_buffer(&message_buffer)?;
                // Read the resulting data as the name
                let received_buffer = self.enclave.receive_buffer()?;
                let received_message: RuntimeManagerMessage = bincode::deserialize(&received_buffer)?;
                match received_message {
                    RuntimeManagerMessage::EnclaveName(name) => name,
                    _ => return Err(SinaloaError::InvalidRuntimeManagerMessage(received_message)),
                }
            };
            return Ok(name);
        }

        fn proxy_psa_attestation_get_token(
            &self,
            challenge: Vec<u8>,
        ) -> Result<(Vec<u8>, Vec<u8>, i32), SinaloaError> {
            let message = RuntimeManagerMessage::GetPSAAttestationToken(challenge);
            let message_buffer = bincode::serialize(&message)?;
            self.enclave.send_buffer(&message_buffer)?;

            let received_buffer = self.enclave.receive_buffer()?;
            let received_message: RuntimeManagerMessage = bincode::deserialize(&received_buffer)?;
            let (token, public_key, device_id) = match received_message {
                RuntimeManagerMessage::PSAAttestationToken(token, public_key, device_id) => {
                    (token, public_key, device_id)
                }
                _ => return Err(SinaloaError::InvalidRuntimeManagerMessage(received_message)),
            };
            return Ok((token, public_key, device_id));
        }

        fn new_tls_session(&self) -> Result<u32, SinaloaError> {
            let nls_message = RuntimeManagerMessage::NewTLSSession;
            let nls_buffer = bincode::serialize(&nls_message)?;
            self.enclave.send_buffer(&nls_buffer)?;

            let received_buffer: Vec<u8> = self.enclave.receive_buffer()?;

            let received_message: RuntimeManagerMessage = bincode::deserialize(&received_buffer)?;
            let session_id = match received_message {
                RuntimeManagerMessage::TLSSession(sid) => sid,
                _ => return Err(SinaloaError::InvalidRuntimeManagerMessage(received_message)),
            };
            return Ok(session_id);
        }

        fn close_tls_session(&self, session_id: u32) -> Result<(), SinaloaError> {
            let cts_message = RuntimeManagerMessage::CloseTLSSession(session_id);
            let cts_buffer = bincode::serialize(&cts_message)?;

            self.enclave.send_buffer(&cts_buffer)?;

            let received_buffer: Vec<u8> = self.enclave.receive_buffer()?;

            let received_message: RuntimeManagerMessage = bincode::deserialize(&received_buffer)?;
            return match received_message {
                RuntimeManagerMessage::Status(_status) => Ok(()),
                _ => Err(SinaloaError::NitroStatus(NitroStatus::Fail)),
            };
        }

        fn tls_data(
            &self,
            session_id: u32,
            input: Vec<u8>,
        ) -> Result<(bool, Option<Vec<Vec<u8>>>), SinaloaError> {
            let std_message: RuntimeManagerMessage = RuntimeManagerMessage::SendTLSData(session_id, input);
            let std_buffer: Vec<u8> = bincode::serialize(&std_message)?;

            self.enclave.send_buffer(&std_buffer)?;

            let received_buffer: Vec<u8> = self.enclave.receive_buffer()?;

            let received_message: RuntimeManagerMessage = bincode::deserialize(&received_buffer)?;
            match received_message {
                RuntimeManagerMessage::Status(status) => match status {
                    NitroStatus::Success => (),
                    _ => return Err(SinaloaError::NitroStatus(status)),
                },
                _ => return Err(SinaloaError::InvalidRuntimeManagerMessage(received_message)),
            }

            let mut active_flag = true;
            let mut ret_array = Vec::new();
            while self.tls_data_needed(session_id)? {
                let gtd_message = RuntimeManagerMessage::GetTLSData(session_id);
                let gtd_buffer: Vec<u8> = bincode::serialize(&gtd_message)?;

                self.enclave.send_buffer(&gtd_buffer)?;

                let received_buffer: Vec<u8> = self.enclave.receive_buffer()?;

                let received_message: RuntimeManagerMessage = bincode::deserialize(&received_buffer)?;
                match received_message {
                    RuntimeManagerMessage::TLSData(data, alive) => {
                        active_flag = alive;
                        ret_array.push(data);
                    }
                    _ => return Err(SinaloaError::NitroStatus(NitroStatus::Fail)),
                }
            }

            Ok((
                active_flag,
                if ret_array.len() > 0 {
                    Some(ret_array)
                } else {
                    None
                },
            ))
        }

        fn close(&mut self) -> Result<bool, SinaloaError> {
            let re_message: RuntimeManagerMessage = RuntimeManagerMessage::ResetEnclave;
            let re_buffer: Vec<u8> = bincode::serialize(&re_message)?;

            self.enclave.send_buffer(&re_buffer)?;

            let received_buffer: Vec<u8> = self.enclave.receive_buffer()?;
            let received_message: RuntimeManagerMessage = bincode::deserialize(&received_buffer)?;
            return match received_message {
                RuntimeManagerMessage::Status(status) => match status {
                    NitroStatus::Success => Ok(true),
                    _ => Err(SinaloaError::NitroStatus(status)),
                },
                _ => Err(SinaloaError::InvalidRuntimeManagerMessage(received_message)),
            };
        }
    }

    impl Drop for SinaloaNitro {
        fn drop(&mut self) {
            match self.close() {
                Err(err) => println!("SinaloaNitro::drop failed in call to self.close:{:?}, we will persevere, though.", err),
                _ => (),
            }
        }
    }

    impl SinaloaNitro {
        fn sinaloa_ocall_handler(input_buffer: Vec<u8>) -> Result<Vec<u8>, NitroError> {
            let return_buffer: Vec<u8> = {
                let mut nre_guard = NRE_CONTEXT.lock().map_err(|_| NitroError::MutexError)?;
                match &mut *nre_guard {
                    Some(nre) => {
                        nre.send_buffer(&input_buffer).map_err(|err| {
                            println!(
                                "SinaloaNitro::sinaloa_ocall_handler send_buffer failed:{:?}",
                                err
                            );
                            NitroError::EC2Error
                        })?;
                        let ret_buffer = nre.receive_buffer().map_err(|err| {
                            println!(
                                "SinaloaNitro::sinaloa_ocall_handler receive_buffer failed:{:?}",
                                err
                            );
                            NitroError::EC2Error
                        })?;
                        ret_buffer
                    }
                    None => return Err(NitroError::EC2Error),
                }
            };
            return Ok(return_buffer);
        }

        fn tls_data_needed(&self, session_id: u32) -> Result<bool, SinaloaError> {
            let gtdn_message = RuntimeManagerMessage::GetTLSDataNeeded(session_id);
            let gtdn_buffer: Vec<u8> = bincode::serialize(&gtdn_message)?;

            self.enclave.send_buffer(&gtdn_buffer)?;

            let received_buffer: Vec<u8> = self.enclave.receive_buffer()?;

            let received_message: RuntimeManagerMessage = bincode::deserialize(&received_buffer)?;
            let tls_data_needed = match received_message {
                RuntimeManagerMessage::TLSDataNeeded(needed) => needed,
                _ => return Err(SinaloaError::NitroStatus(NitroStatus::Fail)),
            };
            return Ok(tls_data_needed);
        }

        fn native_attestation(
            proxy_attestation_server_url: &str,
            _runtime_manager_hash: &str,
            //) -> Result<NitroEnclave, SinaloaError> {
        ) -> Result<EC2Instance, SinaloaError> {
            println!("SinaloaNitro::native_attestation started");

            println!("Starting EC2 instance");
            let nre_instance = EC2Instance::new().map_err(|err| SinaloaError::EC2Error(err))?;

            nre_instance
                .upload_file(
                    NITRO_ROOT_ENCLAVE_EIF_PATH,
                    "/home/ec2-user/nitro_root_enclave.eif",
                )
                .map_err(|err| SinaloaError::EC2Error(err))?;
            nre_instance
                .upload_file(
                    NITRO_ROOT_ENCLAVE_SERVER_PATH,
                    "/home/ec2-user/nitro-root-enclave-server",
                )
                .map_err(|err| SinaloaError::EC2Error(err))?;

            nre_instance
                .execute_command("nitro-cli-config -t 2 -m 512")
                .map_err(|err| SinaloaError::EC2Error(err))?;
            #[cfg(feature = "debug")]
            let server_command: String = format!(
                "nohup /home/ec2-user/nitro-root-enclave-server --debug {:} &> nitro_server.log &",
                proxy_attestation_server_url
            );
            #[cfg(not(feature = "debug"))]
            let server_command: String = format!(
                "nohup /home/ec2-user/nitro-root-enclave-server {:} &> nitro_server.log &",
                proxy_attestation_server_url
            );
            nre_instance
                .execute_command(&server_command)
                .map_err(|err| SinaloaError::EC2Error(err))?;

            println!("Waiting for NRE Instance to authenticate.");
            std::thread::sleep(std::time::Duration::from_millis(15000));

            println!("sinaloa_tz::native_attestation returning Ok");
            return Ok(nre_instance);
        }
    }
}
