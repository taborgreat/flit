#![no_main]
//! HostState passthrough: any blob goes in, the same blob comes out of
//! replay, and the chains never panic on it.
use flit_core::*;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let key = Keypair::from_seed(1);
    let host = HostTag::new(if data.first().map(|b| b % 2 == 0).unwrap_or(true) { "a" } else { "b" });
    let mut log = Chains::in_memory();
    let main = stamp::main_branch();
    let genesis = Stamp::sign(
        StampBody { name: key.id(), act_prev: None, thing: Thing::Song, fact_prev: vec![], branch: main, gate: false, host: host.clone(), wall: Time::at(0, SongPos::START), payload: Payload::Crew(CrewAction::Approve { id: key.id(), owner: true }) },
        &key,
    );
    log.append(genesis.clone()).unwrap();
    let lane = LaneId::new("l");
    let ev = Stamp::sign(
        StampBody { name: key.id(), act_prev: Some(genesis.id), thing: Thing::Lane(lane.clone()), fact_prev: vec![], branch: main, gate: false, host: host.clone(), wall: Time::at(1, SongPos::START), payload: Payload::HostState(HostStatePayload { host: host.clone(), blob: data.to_vec() }) },
        &key,
    );
    match log.append(ev) {
        Ok(AppendOutcome::Appended) => {
            let st = log.state(&main).unwrap();
            assert_eq!(st.lane(&lane).unwrap().host_state.get(&host).map(|v| v.as_slice()), Some(data));
        }
        Ok(_) => unreachable!(),
        Err(Error::Invalid(_)) => assert!(data.len() > flit_core::seal::verify::MAX_HOST_STATE),
        Err(e) => panic!("{e}"),
    }
});
