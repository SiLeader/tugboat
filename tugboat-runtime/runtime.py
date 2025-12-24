import argparse
import os
import pathlib
import subprocess


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('image', help='Qcow2 image to use for the virtual machine')
    parser.add_argument('--id', '-i', help='Unique identifier for the virtual machine', required=True)

    parser.add_argument('--arch', help='Architecture for the virtual machine', choices=['x86_64'], default='x86_64')
    parser.add_argument('--cores', help='Number of CPU cores for the virtual machine', type=int, default=1)
    parser.add_argument('--sockets', help='Number of CPU sockets for the virtual machine', type=int, default=1)
    parser.add_argument('--dies', help='Number of dies for the virtual machine', type=int, default=1)
    parser.add_argument('--threads', help='Number of threads per core for the virtual machine', type=int, default=1)

    parser.add_argument('--memory', help='Amount of memory in MiB for the virtual machine', type=int, default=1024)

    parser.add_argument('--executable', help='Path to tugboat runtime executable',
                        default=f'{pathlib.Path(__file__).parent.parent}/target/debug/tugboat-lowlevel-runtime')
    parser.add_argument('--config', help='Path to tugboat config file',
                        default=f'{pathlib.Path(__file__).parent}/sample-config.toml')

    args = parser.parse_args()
    os.execve(
        args.executable,
        [args.executable, args.config],
        {
            'TUGBOAT_RUNTIME_VM_ID': args.id,
            'TUGBOAT_RUNTIME_IMAGE': args.image,
            'TUGBOAT_RUNTIME_CPU_ARCH': args.arch,
            'TUGBOAT_RUNTIME_CPU_CORES': str(args.cores),
            'TUGBOAT_RUNTIME_CPU_SOCKETS': str(args.sockets),
            'TUGBOAT_RUNTIME_CPU_DIES': str(args.dies),
            'TUGBOAT_RUNTIME_CPU_THREADS': str(args.threads),
            'TUGBOAT_RUNTIME_MEMORY_SIZE': str(args.memory * 1024 * 1024),
            'RUST_LOG': 'debug',
        }
    )


if __name__ == '__main__':
    main()
