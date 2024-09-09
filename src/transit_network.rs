//constructs and preprocesses the graph struct from OSM data
use crate::coord_int_convert::coord_to_int;
use gtfs_structures::*;
use std::{
    collections::{HashMap, HashSet},
    hash::Hash,
};

use crate::NodeType;

#[derive(Debug, PartialEq, Hash, Eq, Clone, Copy, PartialOrd, Ord)]
pub struct NodeId {
    //0 = "untyped"    1 = "arrival"   2 = "transfer"  3 = "departure"
    pub node_type: NodeType,
    pub station_id: i64,
    pub time: Option<u64>,
    pub trip_id: u64,
    pub lat: i64, //f64 * f64::powi(10.0, 14) as i64
    pub lon: i64, //f64 * f64::powi(10.0, 14) as i64
}

pub fn read_from_gtfs_zip(path: &str) -> Gtfs {
    let gtfs = gtfs_structures::GtfsReader::default()
        .read_shapes(false) // Won’t read shapes to save time and memory
        .read(path)
        .ok();
    gtfs.unwrap()
}

pub fn calendar_date_filter(
    given_weekday: &str,
    service_id: &str,
    calendar: &Calendar,
) -> Option<String> {
    let day_is_valid = match given_weekday {
        "monday" => calendar.monday,
        "tuesday" => calendar.tuesday,
        "wednesday" => calendar.wednesday,
        "thursday" => calendar.thursday,
        "friday" => calendar.friday,
        "saturday" => calendar.saturday,
        "sunday" => calendar.saturday,
        _ => false,
    };

    if day_is_valid {
        Some(service_id.to_owned())
    } else {
        None
    }
}

#[derive(Debug, PartialEq, Clone)]
pub struct TimeExpandedGraph {
    //graph struct that will be used to route
    pub day_of_week: String,
    pub transfer_buffer: u64,
    pub nodes: HashSet<NodeId>,
    pub edges: HashMap<NodeId, HashMap<NodeId, u64>>, // tail.id, <head.id, cost>
    pub station_mapping: HashMap<String, i64>, //station_id string, internal station_id (assigned number)
    pub nodes_per_station: HashMap<i64, Vec<(u64, NodeId)>>,
    pub trip_mapping: HashMap<u64, String>, //internal trip_id (assigned number), trip_id string
}

#[derive(Debug, PartialEq, Clone)]
pub struct LineConnectionTable {
    //node that references parent nodes, used to create path from goal node to start node
    pub route_id: String,
    pub times_from_start: HashMap<i64, (u64, u16)>, //<stationid, (time from start, sequence number)>
    pub start_times: Vec<u64>,                      //start times for vehicle from first station
}

#[derive(Debug, PartialEq, Clone)]
pub struct DirectConnections {
    pub route_tables: HashMap<String, LineConnectionTable>, //route_id, table
    pub lines_per_station: HashMap<i64, HashMap<String, u16>>, //<stationid, <routeid, stop sequence#>>
}

impl TimeExpandedGraph {
    pub fn new(
        mut gtfs: Gtfs,
        mut day_of_week: String,
        transfer_buffer: u64,
    ) -> (Self, DirectConnections) {
        day_of_week = day_of_week.to_lowercase();
        //init new transit network graph based on results from reading GTFS zip
        let mut nodes: HashSet<NodeId> = HashSet::new(); //maps GTFS stop id string to sequential numeric stop id
        let mut edges: HashMap<NodeId, HashMap<NodeId, u64>> = HashMap::new();
        let mut station_mapping: HashMap<String, i64> = HashMap::new();
        let mut nodes_per_station: HashMap<i64, Vec<(u64, NodeId)>> = HashMap::new(); // <stationid, (time, node_id)>, # of stations and # of times
        let mut connection_table_per_line: HashMap<String, LineConnectionTable> = HashMap::new();
        let mut lines_per_station: HashMap<i64, HashMap<String, u16>> = HashMap::new();
        let mut trip_mapping: HashMap<u64, String> = HashMap::new();

        let service_ids_of_given_day: HashSet<String> = gtfs
            .calendar
            .iter()
            .filter_map(|(service_id, calendar)| {
                calendar_date_filter(day_of_week.as_str(), service_id, calendar)
            })
            .collect();

        let trip_ids_of_given_day: HashSet<String> = gtfs
            .trips
            .iter()
            .filter(|(_, trip)| service_ids_of_given_day.contains(&trip.service_id))
            .map(|(trip_id, _)| trip_id.to_owned())
            .collect();

        //TODO: add repetitions of trip_id for frequencies.txt if it exists

        for (iterator, stop_id) in (0_i64..).zip(gtfs.stops.iter()) {
            station_mapping.insert(stop_id.0.clone(), iterator);
        }

        let mut trip_id: u64 = 0; //custom counter like with stop_id
        let mut nodes_by_time: Vec<(u64, NodeId)> = Vec::new();

        for (_, trip) in gtfs.trips.iter_mut() {
            if !trip_ids_of_given_day.contains(&trip.id) {
                continue;
            }

            trip_mapping.insert(trip_id, trip.id.clone());

            let mut id;

            let mut prev_departure: Option<(NodeId, u64)> = None;

            trip.stop_times
                .sort_by(|a, b| a.stop_sequence.cmp(&b.stop_sequence));

            let trip_start_time: u64 = trip
                .stop_times
                .first()
                .unwrap()
                .arrival_time
                .unwrap()
                .into();
            let mut stations_time_from_trip_start = HashMap::new();

            for stoptime in trip.stop_times.iter() {
                id = *station_mapping.get(&stoptime.stop.id).unwrap();

                //write a function that traces up parent stations for lat and lon if unwrap fails (optional value)
                //if let Some(other_stop_id) = stoptime.stop.parent_station {
                //
                //} else{
                let (lon, lat) = coord_to_int(
                    stoptime.stop.longitude.unwrap(),
                    stoptime.stop.latitude.unwrap(),
                );
                //}

                let arrival_time: u64 = stoptime.arrival_time.unwrap().into();
                let departure_time: u64 = stoptime.departure_time.unwrap().into();

                stations_time_from_trip_start
                    .insert(id, (arrival_time - trip_start_time, stoptime.stop_sequence));

                let arrival_node = NodeId {
                    node_type: NodeType::Arrival,
                    station_id: id,
                    time: Some(arrival_time),
                    trip_id,
                    lat,
                    lon,
                };
                let transfer_node = NodeId {
                    node_type: NodeType::Transfer,
                    station_id: id,
                    time: Some(arrival_time + transfer_buffer),
                    trip_id,
                    lat,
                    lon,
                };
                let departure_node = NodeId {
                    node_type: NodeType::Departure,
                    station_id: id,
                    time: Some(departure_time),
                    trip_id,
                    lat,
                    lon,
                };

                nodes.insert(arrival_node);
                nodes.insert(transfer_node);
                nodes.insert(departure_node);

                if let Some((prev_dep, prev_dep_time)) = prev_departure {
                    edges //travelling arc for previous departure to current arrival
                        .entry(prev_dep) //tail
                        .and_modify(|inner| {
                            inner.insert(arrival_node, arrival_time - prev_dep_time);
                            //head
                        })
                        .or_insert({
                            let mut a = HashMap::new();
                            a.insert(arrival_node, arrival_time - prev_dep_time); //head
                            a
                        });
                }

                edges //layover arc for current arrival to current departure
                    .entry(arrival_node) //tail
                    .and_modify(|inner| {
                        inner.insert(departure_node, departure_time - arrival_time);
                        //head
                    })
                    .or_insert({
                        let mut a = HashMap::new();
                        a.insert(departure_node, departure_time - arrival_time); //head
                        a
                    });

                edges //alighting arc (arrival to transfer)
                    .entry(arrival_node) //tail
                    .and_modify(|inner| {
                        inner.insert(transfer_node, transfer_buffer);
                        //head
                    })
                    .or_insert({
                        let mut a = HashMap::new();
                        a.insert(transfer_node, transfer_buffer); //head
                        a
                    });

                let node_list = vec![
                    (arrival_time, arrival_node),
                    (arrival_time + transfer_buffer, transfer_node),
                    (departure_time, departure_node),
                ];

                nodes_by_time.extend(node_list.iter());

                nodes_per_station
                    .entry(id)
                    .and_modify(|inner| {
                        inner.extend(node_list.iter());
                    })
                    .or_insert(node_list);

                prev_departure = Some((departure_node, departure_time));
            }

            trip_id += 1;
            let route_id = trip.route_id.clone();

            connection_table_per_line
                .entry(route_id.clone())
                .and_modify(|table| {
                    table.route_id = route_id.clone();
                    table.start_times.push(trip_start_time);
                    table
                        .times_from_start
                        .extend(stations_time_from_trip_start.iter());
                })
                .or_insert({
                    LineConnectionTable {
                        route_id,
                        start_times: Vec::from([trip_start_time]),
                        times_from_start: stations_time_from_trip_start,
                    }
                });
        }
        for (station_id, station) in nodes_per_station.iter_mut() {
            station.sort_by(|a, b| a.0.cmp(&b.0));
            let time_chunks = station.chunk_by_mut(|a, b| a.0 == b.0);

            let mut station_nodes_by_time: Vec<(u64, NodeId)> = Vec::new();
            for chunk in time_chunks {
                chunk.sort_by(|a, b| a.1.node_type.cmp(&b.1.node_type));
                station_nodes_by_time.append(&mut chunk.to_vec().to_owned())
            }

            for (current_index, node) in station_nodes_by_time.iter().enumerate() {
                if node.1.node_type == NodeType::Transfer {
                    for index in current_index + 1..station_nodes_by_time.len() {
                        let future_node = station_nodes_by_time.get(index).unwrap();
                        if future_node.1.node_type == NodeType::Transfer {
                            edges //waiting arc (transfer to transfer)
                                .entry(node.1) //tail
                                .and_modify(|inner| {
                                    inner.insert(future_node.1, future_node.0 - node.0);
                                    //head
                                })
                                .or_insert({
                                    let mut a = HashMap::new();
                                    a.insert(future_node.1, future_node.0 - node.0); //head
                                    a
                                });
                            break;
                        }

                        if future_node.1.node_type == NodeType::Departure {
                            edges //boarding arc (transfer to departure)
                                .entry(node.1) //tail
                                .and_modify(|inner| {
                                    inner.insert(future_node.1, future_node.0 - node.0);
                                    //head
                                })
                                .or_insert({
                                    let mut a = HashMap::new();
                                    a.insert(future_node.1, future_node.0 - node.0); //head
                                    a
                                });
                        }
                    }
                }
            }
            for (route_id, line) in connection_table_per_line.iter() {
                if let Some((_, sequence_number)) = line.times_from_start.get(station_id) {
                    lines_per_station
                        .entry(*station_id)
                        .and_modify(|map| {
                            map.insert(route_id.clone(), *sequence_number);
                        })
                        .or_insert({
                            let mut map = HashMap::new();
                            map.insert(route_id.clone(), *sequence_number);
                            map
                        });
                }
            }
        }

        (
            Self {
                day_of_week,
                transfer_buffer,
                nodes,
                edges,
                station_mapping,
                nodes_per_station,
                trip_mapping,
            },
            DirectConnections {
                route_tables: connection_table_per_line,
                lines_per_station,
            },
        )
    }

    //removed visited nodes as part of graph struct, so this no longer works and we're not using it anyway so it can just sit here
    /*
    pub fn reduce_to_largest_connected_component(self) -> Self {
        let saved_day = self.day_of_week.clone();
        let saved_tb = self.transfer_buffer;
        //reduces graph to largest connected component through nodes visited with time_expanded_dijkstra
        let mut counter = 0;
        let mut number_times_node_visted: HashMap<NodeId, i32> = HashMap::new();
        let shortest_path_graph = TransitDijkstra::new(&self);

        while let Some(source_id) =
            shortest_path_graph.get_unvisted_node_id(&number_times_node_visted)
        {
            counter += 1;
            let mut shortest_path_graph = TransitDijkstra::new(&self);
            shortest_path_graph.time_expanded_dijkstra(Some(source_id), None, None, None);
            for node in shortest_path_graph.visited_nodes.keys() {
                number_times_node_visted.insert(*node, counter);
            }
            if number_times_node_visted.len() > (self.nodes.len() / 2) {
                break;
            }
        }

        let mut new_node_list: Vec<(&NodeId, &i32)> = number_times_node_visted.iter().collect();
        new_node_list.sort_by(|(_, counter1), (_, counter2)| counter1.cmp(counter2));

        let connected_components =
            &mut new_node_list.chunk_by(|(_, counter1), (_, counter2)| counter1 == counter2);

        let mut largest_node_set = Vec::new();
        let mut prev_set_size = 0;

        for node_set in connected_components.by_ref() {
            if node_set.len() > prev_set_size {
                largest_node_set = node_set.to_vec();
                prev_set_size = node_set.len();
            }
        }

        let lcc_nodes = largest_node_set
            .iter()
            .map(|(id, _)| (**id))
            .collect::<HashSet<NodeId>>();

        let mut filtered_edges = HashMap::new();

        for (tail, edge) in self.edges {
            let mut inner = HashMap::new();
            for (head, info) in edge {
                if lcc_nodes.contains(&head) {
                    inner.insert(head, info);
                }
            }
            if lcc_nodes.contains(&tail) {
                filtered_edges.insert(tail, inner);
            }
        }

        Self {
            day_of_week: saved_day,
            transfer_buffer: saved_tb,
            nodes: lcc_nodes,
            edges: filtered_edges,
            station_mapping: self.station_mapping,
            nodes_per_station: self.nodes_per_station,
            trip_mapping: self.trip_mapping,
        }
    }*/
}

//For each station: Hashmap<stationid, (line, stop sequence #)>
//Line struct: line_id, hashmap<station, time_from_start>, hashset<starttime>
//Given time and connection: find intersection of route of two stations where
//(station line start) > (station line end)
//find (first start time) after given time - (hashmap find start)
//compute arrival time from (first start time) + (hashmap find end)

pub fn direct_connection_query(
    connections: &DirectConnections,
    start_station: i64,
    end_station: i64,
    time: u64,
) -> Option<(u64, u64)> {
    //departure time from start, arrival time to end
    let start = connections.lines_per_station.get(&start_station).unwrap();
    let end = connections.lines_per_station.get(&end_station).unwrap();

    let mut route = "";
    for (s_route, s_seq) in start {
        for (e_route, e_seq) in end {
            if s_route == e_route && s_seq < e_seq {
                route = s_route;
                break;
            }
        }
    }

    let table = connections.route_tables.get(route).unwrap();
    let mut start_times = table.start_times.clone();
    let time_to_start = table.times_from_start.get(&start_station).unwrap().0;
    let time_to_end = table.times_from_start.get(&end_station).unwrap().0;
    start_times.sort();
    if let Some(first_valid_start_time) = start_times.iter().find(|&&s| s > (time - time_to_start))
    {
        let departure = first_valid_start_time + time_to_start;
        let arrival = first_valid_start_time + time_to_end;
        Some((departure, arrival))
    } else {
        None
    }
}
