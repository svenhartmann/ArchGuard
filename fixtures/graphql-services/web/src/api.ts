import { gql } from "@apollo/client";

const USER_FIELDS = gql`
  fragment UserFields on User { id name }
`;

export const GET_USER = gql`
  query GetUser($id: ID!) { user(id: $id) { ...UserFields } }
  ${USER_FIELDS}
`;

export const GET_PRODUCT = gql`query GetProduct { p: product(id: "1") { id title } }`;

// Both subgraphs declare Query.node: no provider is guessed.
export const NODE = gql`query Node($id: ID!) { node(id: $id) { id } }`;

export const ORDERS = gql`query Orders { orders { id } }`;
