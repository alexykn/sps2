from setuptools import setup

setup(
    name='hello-setuppy',
    version='1.0.0',
    py_modules=['hello'],
    entry_points={
        'console_scripts': [
            'hello-setuppy=hello:main',
        ],
    },
)