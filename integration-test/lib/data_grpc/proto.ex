defmodule DataGRPC.DataAPIResult.Status do
  @moduledoc false

  use Protobuf, enum: true, protoc_gen_elixir_version: "0.13.0", syntax: :proto3

  def descriptor do
    # credo:disable-for-next-line
    %Google.Protobuf.EnumDescriptorProto{
      __unknown_fields__: [],
      name: "Status",
      options: nil,
      reserved_name: [],
      reserved_range: [],
      value: [
        %Google.Protobuf.EnumValueDescriptorProto{
          __unknown_fields__: [],
          name: "OK",
          number: 0,
          options: nil
        },
        %Google.Protobuf.EnumValueDescriptorProto{
          __unknown_fields__: [],
          name: "ERROR",
          number: 1,
          options: nil
        }
      ]
    }
  end

  field(:OK, 0)
  field(:ERROR, 1)
end

defmodule DataGRPC.ValidateJWTResult.Status do
  @moduledoc false

  use Protobuf, enum: true, protoc_gen_elixir_version: "0.13.0", syntax: :proto3

  def descriptor do
    # credo:disable-for-next-line
    %Google.Protobuf.EnumDescriptorProto{
      __unknown_fields__: [],
      name: "Status",
      options: nil,
      reserved_name: [],
      reserved_range: [],
      value: [
        %Google.Protobuf.EnumValueDescriptorProto{
          __unknown_fields__: [],
          name: "OK",
          number: 0,
          options: nil
        },
        %Google.Protobuf.EnumValueDescriptorProto{
          __unknown_fields__: [],
          name: "ERROR",
          number: 1,
          options: nil
        }
      ]
    }
  end

  field(:OK, 0)
  field(:ERROR, 1)
end

defmodule DataGRPC.Context do
  @moduledoc false

  use Protobuf, protoc_gen_elixir_version: "0.13.0", syntax: :proto3

  def descriptor do
    # credo:disable-for-next-line
    %Google.Protobuf.DescriptorProto{
      __unknown_fields__: [],
      enum_type: [],
      extension: [],
      extension_range: [],
      field: [
        %Google.Protobuf.FieldDescriptorProto{
          __unknown_fields__: [],
          default_value: nil,
          extendee: nil,
          json_name: "applicationId",
          label: :LABEL_OPTIONAL,
          name: "application_id",
          number: 1,
          oneof_index: nil,
          options: nil,
          proto3_optional: nil,
          type: :TYPE_STRING,
          type_name: nil
        },
        %Google.Protobuf.FieldDescriptorProto{
          __unknown_fields__: [],
          default_value: nil,
          extendee: nil,
          json_name: "jwt",
          label: :LABEL_OPTIONAL,
          name: "jwt",
          number: 2,
          oneof_index: nil,
          options: nil,
          proto3_optional: nil,
          type: :TYPE_STRING,
          type_name: nil
        }
      ],
      name: "Context",
      nested_type: [],
      oneof_decl: [],
      options: nil,
      reserved_name: [],
      reserved_range: []
    }
  end

  field(:application_id, 1, type: :string, json_name: "applicationId")
  field(:jwt, 2, type: :string)
end

defmodule DataGRPC.DataAPIRequest do
  @moduledoc false

  use Protobuf, protoc_gen_elixir_version: "0.13.0", syntax: :proto3

  def descriptor do
    # credo:disable-for-next-line
    %Google.Protobuf.DescriptorProto{
      __unknown_fields__: [],
      enum_type: [],
      extension: [],
      extension_range: [],
      field: [
        %Google.Protobuf.FieldDescriptorProto{
          __unknown_fields__: [],
          default_value: nil,
          extendee: nil,
          json_name: "query",
          label: :LABEL_OPTIONAL,
          name: "query",
          number: 1,
          oneof_index: nil,
          options: nil,
          proto3_optional: nil,
          type: :TYPE_STRING,
          type_name: nil
        },
        %Google.Protobuf.FieldDescriptorProto{
          __unknown_fields__: [],
          default_value: nil,
          extendee: nil,
          json_name: "variables",
          label: :LABEL_OPTIONAL,
          name: "variables",
          number: 2,
          oneof_index: nil,
          options: nil,
          proto3_optional: nil,
          type: :TYPE_STRING,
          type_name: nil
        },
        %Google.Protobuf.FieldDescriptorProto{
          __unknown_fields__: [],
          default_value: nil,
          extendee: nil,
          json_name: "context",
          label: :LABEL_OPTIONAL,
          name: "context",
          number: 3,
          oneof_index: nil,
          options: nil,
          proto3_optional: nil,
          type: :TYPE_MESSAGE,
          type_name: ".data_grpc.Context"
        }
      ],
      name: "DataAPIRequest",
      nested_type: [],
      oneof_decl: [],
      options: nil,
      reserved_name: [],
      reserved_range: []
    }
  end

  field(:query, 1, type: :string)
  field(:variables, 2, type: :string)
  field(:context, 3, type: DataGRPC.Context)
end

defmodule DataGRPC.ValidateJWTRequest do
  @moduledoc false

  use Protobuf, protoc_gen_elixir_version: "0.13.0", syntax: :proto3

  def descriptor do
    # credo:disable-for-next-line
    %Google.Protobuf.DescriptorProto{
      __unknown_fields__: [],
      enum_type: [],
      extension: [],
      extension_range: [],
      field: [
        %Google.Protobuf.FieldDescriptorProto{
          __unknown_fields__: [],
          default_value: nil,
          extendee: nil,
          json_name: "context",
          label: :LABEL_OPTIONAL,
          name: "context",
          number: 1,
          oneof_index: nil,
          options: nil,
          proto3_optional: nil,
          type: :TYPE_MESSAGE,
          type_name: ".data_grpc.Context"
        }
      ],
      name: "ValidateJWTRequest",
      nested_type: [],
      oneof_decl: [],
      options: nil,
      reserved_name: [],
      reserved_range: []
    }
  end

  field(:context, 1, type: DataGRPC.Context)
end

defmodule DataGRPC.DataAPIResult do
  @moduledoc false

  use Protobuf, protoc_gen_elixir_version: "0.13.0", syntax: :proto3

  def descriptor do
    # credo:disable-for-next-line
    %Google.Protobuf.DescriptorProto{
      __unknown_fields__: [],
      enum_type: [
        %Google.Protobuf.EnumDescriptorProto{
          __unknown_fields__: [],
          name: "Status",
          options: nil,
          reserved_name: [],
          reserved_range: [],
          value: [
            %Google.Protobuf.EnumValueDescriptorProto{
              __unknown_fields__: [],
              name: "OK",
              number: 0,
              options: nil
            },
            %Google.Protobuf.EnumValueDescriptorProto{
              __unknown_fields__: [],
              name: "ERROR",
              number: 1,
              options: nil
            }
          ]
        }
      ],
      extension: [],
      extension_range: [],
      field: [
        %Google.Protobuf.FieldDescriptorProto{
          __unknown_fields__: [],
          default_value: nil,
          extendee: nil,
          json_name: "status",
          label: :LABEL_OPTIONAL,
          name: "status",
          number: 1,
          oneof_index: nil,
          options: nil,
          proto3_optional: nil,
          type: :TYPE_ENUM,
          type_name: ".data_grpc.DataAPIResult.Status"
        },
        %Google.Protobuf.FieldDescriptorProto{
          __unknown_fields__: [],
          default_value: nil,
          extendee: nil,
          json_name: "result",
          label: :LABEL_OPTIONAL,
          name: "result",
          number: 2,
          oneof_index: nil,
          options: nil,
          proto3_optional: nil,
          type: :TYPE_STRING,
          type_name: nil
        }
      ],
      name: "DataAPIResult",
      nested_type: [],
      oneof_decl: [],
      options: nil,
      reserved_name: [],
      reserved_range: []
    }
  end

  field(:status, 1, type: DataGRPC.DataAPIResult.Status, enum: true)
  field(:result, 2, type: :string)
end

defmodule DataGRPC.ValidateJWTResult do
  @moduledoc false

  use Protobuf, protoc_gen_elixir_version: "0.13.0", syntax: :proto3

  def descriptor do
    # credo:disable-for-next-line
    %Google.Protobuf.DescriptorProto{
      __unknown_fields__: [],
      enum_type: [
        %Google.Protobuf.EnumDescriptorProto{
          __unknown_fields__: [],
          name: "Status",
          options: nil,
          reserved_name: [],
          reserved_range: [],
          value: [
            %Google.Protobuf.EnumValueDescriptorProto{
              __unknown_fields__: [],
              name: "OK",
              number: 0,
              options: nil
            },
            %Google.Protobuf.EnumValueDescriptorProto{
              __unknown_fields__: [],
              name: "ERROR",
              number: 1,
              options: nil
            }
          ]
        }
      ],
      extension: [],
      extension_range: [],
      field: [
        %Google.Protobuf.FieldDescriptorProto{
          __unknown_fields__: [],
          default_value: nil,
          extendee: nil,
          json_name: "status",
          label: :LABEL_OPTIONAL,
          name: "status",
          number: 1,
          oneof_index: nil,
          options: nil,
          proto3_optional: nil,
          type: :TYPE_ENUM,
          type_name: ".data_grpc.ValidateJWTResult.Status"
        },
        %Google.Protobuf.FieldDescriptorProto{
          __unknown_fields__: [],
          default_value: nil,
          extendee: nil,
          json_name: "result",
          label: :LABEL_OPTIONAL,
          name: "result",
          number: 2,
          oneof_index: nil,
          options: nil,
          proto3_optional: nil,
          type: :TYPE_STRING,
          type_name: nil
        }
      ],
      name: "ValidateJWTResult",
      nested_type: [],
      oneof_decl: [],
      options: nil,
      reserved_name: [],
      reserved_range: []
    }
  end

  field(:status, 1, type: DataGRPC.ValidateJWTResult.Status, enum: true)
  field(:result, 2, type: :string)
end

defmodule DataGRPC.DataAPI.Service do
  @moduledoc false

  use GRPC.Service, name: "data_grpc.DataAPI", protoc_gen_elixir_version: "0.13.0"

  def descriptor do
    # credo:disable-for-next-line
    %Google.Protobuf.ServiceDescriptorProto{
      __unknown_fields__: [],
      method: [
        %Google.Protobuf.MethodDescriptorProto{
          __unknown_fields__: [],
          client_streaming: false,
          input_type: ".data_grpc.DataAPIRequest",
          name: "Execute",
          options: nil,
          output_type: ".data_grpc.DataAPIResult",
          server_streaming: false
        },
        %Google.Protobuf.MethodDescriptorProto{
          __unknown_fields__: [],
          client_streaming: false,
          input_type: ".data_grpc.ValidateJWTRequest",
          name: "ValidateJWT",
          options: nil,
          output_type: ".data_grpc.ValidateJWTResult",
          server_streaming: false
        }
      ],
      name: "DataAPI",
      options: nil
    }
  end

  rpc(:Execute, DataGRPC.DataAPIRequest, DataGRPC.DataAPIResult)

  rpc(:ValidateJWT, DataGRPC.ValidateJWTRequest, DataGRPC.ValidateJWTResult)
end

defmodule DataGRPC.DataAPI.Stub do
  @moduledoc false

  use GRPC.Stub, service: DataGRPC.DataAPI.Service
end
